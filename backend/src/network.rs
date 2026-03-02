use rtnetlink::{Handle, LinkBridge, LinkUnspec, new_connection, packet_route::link::LinkMessage};
use futures_util::stream::TryStreamExt;
use nftables::{
    helper,
    schema,
    batch::Batch,
    stmt::{Statement, Match, Operator},
    expr::{Expression, NamedExpression, Payload, PayloadField, Prefix},
    types,
};
use std::env;
use std::net::Ipv4Addr;
use tracing::info;

/// Set up the bridge interface
pub async fn setup_bridge() -> Result<(), Box<dyn std::error::Error>> {
    let bridge_name = env::var("BRIDGE_NAME").unwrap_or_else(|_| "cloudebr0".to_string());
    let bridge_ip: Ipv4Addr = env::var("BRIDGE_IP").unwrap_or_else(|_| "192.168.39.39".to_string()).parse()?;

    // Create rtnetlink connection
    let (connection, handle, _) = new_connection()?;
    tokio::spawn(connection);

    // Check if bridge already exists
    info!("Checking for existing bridge: {}", bridge_name);
    let link_index = match get_link_by_name(&handle, &bridge_name).await? {
        Some(link) => {
            info!("Bridge {} already exists with index {}", bridge_name, link.header.index);
            link.header.index
        }
        None => {
            info!("Creating new bridge: {}", bridge_name);
            create_bridge(&handle, &bridge_name).await?
        }
    };

    // Configure the bridge
    info!("Adding IP address {} to bridge", bridge_ip);
    match handle
        .address()
        .add(link_index, bridge_ip.into(), 24)
        .execute()
        .await
    {
        Ok(_) => info!("IP address added successfully"),
        // Could have checked NetlinkError but it's way too complicated
        Err(e) if e.to_string().contains("File exists") => {
            info!("IP address already exists on bridge");
        }
        Err(e) => return Err(e.into()),
    }

    info!("enabling bridge interface");
    handle
        .link()
        .set(LinkUnspec::new_with_index(link_index).up().build())
        .execute()
        .await?;

    info!("Bridge {} setup complete", bridge_name);
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

    info!("Created bridge {} with index {}", name, link.header.index);
    Ok(link.header.index)
}

/// Check if NAT table and rules already exist
/// i hate you nftablesd
fn check_nat_rules_exist() -> Result<bool, Box<dyn std::error::Error>> {
    // Get current ruleset
    let nftables = helper::get_current_ruleset()?;

    let has_correct_rule = nftables.objects.iter().any(|object| {
        match object {
            schema::NfObject::ListObject(schema::NfListObject::Rule(rule))
                if rule.family == types::NfFamily::IP
                    && rule.table == "nat"
                    && rule.chain == "POSTROUTING" =>
            {
                let mut has_ip_match = false;
                let mut has_masquerade = false;

                for stmt in rule.expr.iter() {
                    match stmt {
                        Statement::Match(m) => {
                            if let Expression::Named(NamedExpression::Prefix(prefix)) = &m.right {
                                if let Expression::String(addr) = &*prefix.addr {
                                    if addr.as_ref() == "192.168.39.0" && prefix.len == 24 {
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
        }
    });

    Ok(has_correct_rule)
}

/// Set up NAT rules using nftables
pub fn setup_nat() -> Result<(), Box<dyn std::error::Error>> {
    // Check if NAT rules already exist
    if check_nat_rules_exist()? {
        info!("NAT rules already exist, skipping setup");
        return Ok(());
    }

    info!("Setting up NAT rules");
    let mut batch = Batch::new();

    // Create nat table
    batch.add(schema::NfListObject::Table(schema::Table {
        family: types::NfFamily::IP,
        name: "nat".into(),
        ..Default::default()
    }));

    // Create postrouting chain in nat table
    batch.add(schema::NfListObject::Chain(schema::Chain {
        family: types::NfFamily::IP,
        table: "nat".into(),
        name: "POSTROUTING".into(),
        _type: Some(types::NfChainType::NAT),
        hook: Some(types::NfHook::Postrouting),
        prio: Some(1),
        policy: Some(types::NfChainPolicy::Accept),
        ..Default::default()
    }));

    // Add masquerade rule to postrouting chain (only for bridge network traffic)
    batch.add(schema::NfListObject::Rule(schema::Rule {
        family: types::NfFamily::IP,
        table: "nat".into(),
        chain: "POSTROUTING".into(),
        expr: vec![
            Statement::Match(Match {
                left: Expression::Named(NamedExpression::Payload(
                    Payload::PayloadField(PayloadField {
                        protocol: "ip".into(),
                        field: "saddr".into(),
                    })
                )),
                right: Expression::Named(NamedExpression::Prefix(Prefix {
                    addr: Box::new(Expression::String("192.168.39.0".into())),
                    len: 24,
                })),
                op: Operator::EQ,
            }),
            // Action: masquerade
            Statement::Masquerade(None),
        ].into(),
        ..Default::default()
    }));

    helper::apply_ruleset(&batch.to_nftables())?;
    info!("NAT rules setup complete");
    Ok(())
}
