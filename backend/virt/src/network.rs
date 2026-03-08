use futures_util::stream::TryStreamExt;
use nftables::{
    batch::Batch,
    expr::{Expression, NamedExpression, Payload, PayloadField, Prefix},
    helper, schema,
    stmt::{Match, Operator, Statement},
    types,
};
use rtnetlink::{Handle, LinkBridge, LinkUnspec, new_connection, packet_route::link::LinkMessage};
use std::net::Ipv4Addr;
use tracing::{debug};

const NAT_TABLE: &str = "cloude_nat";
const NAT_CHAIN: &str = "cloude_postr";

/// Set up the bridge interface
pub async fn setup_bridge(
    bridge_name: String,
    ip_host: Ipv4Addr,
    ip_mask: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create rtnetlink connection
    let (connection, handle, _) = new_connection()?;
    tokio::spawn(connection);

    // Check if bridge already exists
    debug!("Checking for existing bridge: {}", bridge_name);
    let link_index = match get_link_by_name(&handle, &bridge_name).await? {
        Some(link) => {
            debug!(
                "Bridge {} already exists with index {}",
                bridge_name, link.header.index
            );
            link.header.index
        }
        None => {
            debug!("Creating new bridge: {}", bridge_name);
            create_bridge(&handle, &bridge_name).await?
        }
    };
    
    // gods please pardon me
    let bridge_ip: Ipv4Addr = ip_host.into();

    // Configure the bridge
    debug!("Adding IP address {} to bridge", bridge_ip);
    match handle
        .address()
        .add(link_index, bridge_ip.into(), ip_mask)
        .execute()
        .await
    {
        Ok(_) => debug!("IP address added successfully"),
        // Could have checked NetlinkError but it's way too complicated
        Err(e) if e.to_string().contains("File exists") => {
            debug!("IP address already exists on bridge");
        }
        Err(e) => return Err(e.into()),
    }

    debug!("enabling bridge interface");
    handle
        .link()
        .set(LinkUnspec::new_with_index(link_index).up().build())
        .execute()
        .await?;

    debug!("Bridge {} setup complete", bridge_name);
    Ok(())
}

/// Get a link by name, returns None if not found
async fn get_link_by_name(
    handle: &Handle,
    name: &str,
) -> Result<Option<LinkMessage>, rtnetlink::Error> {
    let mut links = handle.link().get().execute();
    while let Some(link) = links.try_next().await? {
        if let Some(link_name) = link.attributes.iter().find_map(|attr| {
            if let rtnetlink::packet_route::link::LinkAttribute::IfName(n) = attr {
                Some(n.as_str())
            } else {
                None
            }
        }) {
            if link_name == name {
                return Ok(Some(link));
            }
        }
    }

    Ok(None)
}

/// Create a new bridge and return its index
async fn create_bridge(handle: &Handle, name: &str) -> Result<u32, rtnetlink::Error> {
    // Create the bridge
    handle
        .link()
        .add(LinkBridge::new(name).build())
        .execute()
        .await?;

    // Retrieve the newly created bridge
    let link = get_link_by_name(handle, name)
        .await?
        .ok_or_else(|| rtnetlink::Error::RequestFailed)?;

    debug!("Created bridge {} with index {}", name, link.header.index);
    Ok(link.header.index)
}

/// Compute network address for an IPv4 CIDR.
fn network_addr(ip: Ipv4Addr, prefix_len: u8) -> Result<Ipv4Addr, Box<dyn std::error::Error>> {
    if prefix_len > 32 {
        return Err(format!("invalid IPv4 prefix length: {}", prefix_len).into());
    }

    let mask = if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix_len))
    };

    Ok((u32::from(ip) & mask).into())
}

/// Ensure the host allows IPv4 forwarding.
fn ensure_ipv4_forwarding_enabled() -> Result<(), Box<dyn std::error::Error>> {
    const IPV4_FORWARD_PATH: &str = "/proc/sys/net/ipv4/ip_forward";
    let current = std::fs::read_to_string(IPV4_FORWARD_PATH)?;

    if current.trim() == "1" {
        return Ok(());
    }

    std::fs::write(IPV4_FORWARD_PATH, "1\n")?;
    debug!("Enabled IPv4 forwarding on host");
    Ok(())
}

fn nat_table_exists(ruleset: &schema::Nftables) -> bool {
    ruleset.objects.iter().any(|object| match object {
        schema::NfObject::ListObject(schema::NfListObject::Table(table)) => {
            table.family == types::NfFamily::IP && table.name == NAT_TABLE
        }
        _ => false,
    })
}

fn nat_chain_exists(ruleset: &schema::Nftables) -> bool {
    ruleset.objects.iter().any(|object| match object {
        schema::NfObject::ListObject(schema::NfListObject::Chain(chain)) => {
            chain.family == types::NfFamily::IP
                && chain.table == NAT_TABLE
                && chain.name == NAT_CHAIN
        }
        _ => false,
    })
}

/// Check if NAT masquerade rule already exists for the given CIDR.
fn nat_rule_exists(ruleset: &schema::Nftables, cidr_base: Ipv4Addr, prefix_len: u8) -> bool {
    ruleset.objects.iter().any(|object| match object {
        schema::NfObject::ListObject(schema::NfListObject::Rule(rule))
            if rule.family == types::NfFamily::IP
                && rule.table == NAT_TABLE
                && rule.chain == NAT_CHAIN =>
        {
            let mut has_ip_match = false;
            let mut has_masquerade = false;

            for stmt in rule.expr.iter() {
                match stmt {
                    Statement::Match(m) => {
                        if let Expression::Named(NamedExpression::Prefix(prefix)) = &m.right {
                            if let Expression::String(addr) = &*prefix.addr {
                                if addr.as_ref() == cidr_base.to_string()
                                    && prefix.len == u32::from(prefix_len)
                                {
                                    has_ip_match = true;
                                }
                            }
                        }
                    }
                    Statement::Masquerade(_) => has_masquerade = true,
                    _ => {}
                }
            }

            has_ip_match && has_masquerade
        }
        _ => false,
    })
}

/// Set up NAT rules using nftables
pub fn setup_nat(ip_range: Ipv4Addr, ip_mask: u8) -> Result<(), Box<dyn std::error::Error>> {
    let cidr_base = network_addr(ip_range, ip_mask)?;
    ensure_ipv4_forwarding_enabled()?;

    let ruleset = helper::get_current_ruleset()?;
    let table_exists = nat_table_exists(&ruleset);
    let chain_exists = nat_chain_exists(&ruleset);
    let rule_exists = nat_rule_exists(&ruleset, cidr_base, ip_mask);

    if table_exists && chain_exists && rule_exists {
        debug!("NAT rules already exist for {}/{}", cidr_base, ip_mask);
        return Ok(());
    }

    debug!("Setting up NAT rules for {}/{}", cidr_base, ip_mask);
    let mut batch = Batch::new();

    if !table_exists {
        batch.add(schema::NfListObject::Table(schema::Table {
            family: types::NfFamily::IP,
            name: NAT_TABLE.into(),
            ..Default::default()
        }));
    }

    if !chain_exists {
        batch.add(schema::NfListObject::Chain(schema::Chain {
            family: types::NfFamily::IP,
            table: NAT_TABLE.into(),
            name: NAT_CHAIN.into(),
            _type: Some(types::NfChainType::NAT),
            hook: Some(types::NfHook::Postrouting),
            prio: Some(1),
            policy: Some(types::NfChainPolicy::Accept),
            ..Default::default()
        }));
    }

    if !rule_exists {
        batch.add(schema::NfListObject::Rule(schema::Rule {
            family: types::NfFamily::IP,
            table: NAT_TABLE.into(),
            chain: NAT_CHAIN.into(),
            expr: vec![
                Statement::Match(Match {
                    left: Expression::Named(NamedExpression::Payload(Payload::PayloadField(
                        PayloadField {
                            protocol: "ip".into(),
                            field: "saddr".into(),
                        },
                    ))),
                    right: Expression::Named(NamedExpression::Prefix(Prefix {
                        addr: Box::new(Expression::String(cidr_base.to_string().into())),
                        len: u32::from(ip_mask),
                    })),
                    op: Operator::EQ,
                }),
                Statement::Masquerade(None),
            ]
            .into(),
            ..Default::default()
        }));
    }

    helper::apply_ruleset(&batch.to_nftables())?;
    debug!("NAT rules setup complete for {}/{}", cidr_base, ip_mask);
    Ok(())
}

/// setup guest iface to be slave of given bridge
pub async fn setup_guest_iface(
    guest_iface_name: &str,
    bridge_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create rtnetlink connection
    let (connection, handle, _) = new_connection()?;
    tokio::spawn(connection);

    // Get bridge index
    let bridge_index = get_link_by_name(&handle, &bridge_name)
        .await?
        .ok_or_else(|| format!("Bridge {} not found", bridge_name))?
        .header
        .index;

    let guest_iface_index = get_link_by_name(&handle, guest_iface_name)
        .await?
        .ok_or_else(|| format!("Guest interface {} not found", guest_iface_name))?
        .header
        .index;

    // Set iface created by VMM to be slave of bridge
    debug!(
        "Setting guest interface {} as slave of bridge {}",
        guest_iface_name, bridge_name
    );
    handle
        .link()
        .set(
            LinkUnspec::new_with_index(guest_iface_index)
                .controller(bridge_index)
                .build(),
        )
        .execute()
        .await?;

        // Enable the guest iface
    debug!("Enabling guest interface: {}", guest_iface_name);
    handle
        .link()
        .set(LinkUnspec::new_with_name(guest_iface_name).up().build())
        .execute()
        .await?;
    debug!("Guest interface {} setup complete", guest_iface_name);
    Ok(())
}
