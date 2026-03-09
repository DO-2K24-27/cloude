use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use backend::ip_manager::IpManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use virt::network::{setup_bridge, setup_guest_iface, setup_nat};
use vmm::VMM;

// ── Shared application state ────────────────────────────────────────

struct AppState {
    jobs: RwLock<HashMap<String, Job>>,
    /// Pool of IPs assigned to VMs, one per running job.
    ip_manager: Arc<IpManager>,
    /// Pre-built HTTP client reused for all agent calls.
    client: reqwest::Client,
    /// Path to the Linux kernel image used to boot VMs.
    kernel_path: String,
    /// Path to the initramfs that contains the agent binary.
    agent_initramfs: String,
    /// Name of the host bridge VMs are attached to.
    bridge_name: String,
    /// IP of the bridge interface (gateway for VMs).
    host_ip: Ipv4Addr,
    /// Subnet mask (e.g. 255.255.255.0).
    netmask: Ipv4Addr,
    /// Port on which the agent inside each VM listens.
    agent_port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum JobStatus {
    Pending,
    Running,
    Done,
    Error,
}

#[derive(Clone, Debug, Serialize)]
struct Job {
    id: String,
    status: JobStatus,
    language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
    #[serde(skip)]
    created_at: std::time::Instant,
}

// ── Request / Response DTOs ─────────────────────────────────────────

#[derive(Deserialize)]
struct RunRequest {
    language: String,
    code: String,
}

#[derive(Serialize)]
struct RunResponse {
    id: String,
}

// ── Agent DTOs (forwarded to the agent inside the VM) ────────────────

#[derive(Serialize)]
struct AgentExecuteRequest {
    language: String,
    code: String,
}

#[derive(Deserialize)]
struct AgentExecuteResponse {
    #[allow(dead_code)]
    job_id: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

// ── Main ────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // ── Environment ──────────────────────────────────────────────────
    let server_addr =
        env::var("BACKEND_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let bridge_name = env::var("BRIDGE_NAME").unwrap_or_else(|_| "cloudebr0".to_string());

    let kernel_path = env::var("BACKEND_KERNEL_PATH").map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "BACKEND_KERNEL_PATH env variable is required",
        )
    })?;
    let agent_initramfs = env::var("BACKEND_AGENT_INITRAMFS").map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "BACKEND_AGENT_INITRAMFS env variable is required (initramfs with the agent inside)",
        )
    })?;

    let ip_range: Ipv4Addr = env::var("IP_RANGE")
        .unwrap_or_else(|_| "10.39.1.0".to_string())
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("IP_RANGE is invalid: {}", e),
            )
        })?;
    let ip_mask: u8 = env::var("IP_MASK")
        .unwrap_or_else(|_| "24".to_string())
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("IP_MASK is invalid: {}", e),
            )
        })?;
    let agent_port: u16 = env::var("AGENT_PORT")
        .unwrap_or_else(|_| "3001".to_string())
        .parse()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("AGENT_PORT is invalid: {}", e),
            )
        })?;

    // Bridge takes .1, VM pool starts at .2
    let host_ip: Ipv4Addr = (ip_range.to_bits() + 1).into();
    let netmask_bits: u32 = if ip_mask == 0 {
        0
    } else {
        !0u32 << (32 - ip_mask)
    };
    let netmask: Ipv4Addr = netmask_bits.into();
    let pool_start: Ipv4Addr = (ip_range.to_bits() + 2).into();
    let pool_end: Ipv4Addr = (ip_range.to_bits() + (1u32 << (32 - ip_mask)) - 2).into();

    // ── Network setup ────────────────────────────────────────────────
    setup_bridge(bridge_name.clone(), host_ip, ip_mask)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    setup_nat(ip_range, ip_mask)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    // ── IP manager ───────────────────────────────────────────────────
    let ip_manager_path =
        env::var("IP_MANAGER_PATH").unwrap_or_else(|_| "/tmp/cloude_ips.json".to_string());
    let ip_manager = Arc::new(
        IpManager::new(ip_manager_path, pool_start, pool_end)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?,
    );

    // ── HTTP client ──────────────────────────────────────────────────
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .expect("Failed to build HTTP client");

    let state = Arc::new(AppState {
        jobs: RwLock::new(HashMap::new()),
        ip_manager,
        client,
        kernel_path,
        agent_initramfs,
        bridge_name,
        host_ip,
        netmask,
        agent_port,
    });

    // ── Background cleanup: evict finished jobs older than 5 min ────
    const JOB_TTL: Duration = Duration::from_secs(300);
    let cleanup_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let mut jobs = cleanup_state.jobs.write().await;
            let before = jobs.len();
            jobs.retain(|_, j| {
                !matches!(j.status, JobStatus::Done | JobStatus::Error)
                    || j.created_at.elapsed() < JOB_TTL
            });
            let removed = before - jobs.len();
            if removed > 0 {
                info!("Evicted {} expired jobs", removed);
            }
        }
    });

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health_check))
        .route("/run", post(run_job))
        .route("/status/{id}", get(get_status))
        .with_state(state);

    info!("Starting Backend server on {}", &server_addr);
    let listener = TcpListener::bind(&server_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> &'static str {
    "Welcome to the Backend server!"
}

async fn health_check() -> &'static str {
    "Backend server is healthy!"
}

// ── POST /run  –  submit a new job ──────────────────────────────────

async fn run_job(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RunRequest>,
) -> impl IntoResponse {
    let id = uuid::Uuid::new_v4().to_string();

    let job = Job {
        id: id.clone(),
        status: JobStatus::Pending,
        language: payload.language.clone(),
        exit_code: None,
        stdout: None,
        stderr: None,
        created_at: std::time::Instant::now(),
    };

    {
        let mut jobs = state.jobs.write().await;
        jobs.insert(id.clone(), job);
    }

    info!("Job {} created – language={}", id, payload.language);

    let job_id = id.clone();
    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        execute_job_in_vm(state_clone, job_id, payload.language, payload.code).await;
    });

    (StatusCode::ACCEPTED, Json(RunResponse { id }))
}

// ── GET /status/:id  –  query job result ────────────────────────────

async fn get_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let jobs = state.jobs.read().await;

    match jobs.get(&id) {
        Some(job) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": job.id,
                "status": job.status,
                "exit_code": job.exit_code,
                "stdout": job.stdout,
                "stderr": job.stderr,
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Job {id} not found"),
            })),
        ),
    }
}

// ── VM lifecycle ─────────────────────────────────────────────────────

/// Entry-point for the background task that handles a single job:
/// spin up a VM, forward code to the agent inside, collect the result.
async fn execute_job_in_vm(
    state: Arc<AppState>,
    job_id: String,
    language: String,
    code: String,
) {
    // Mark job as running
    {
        let mut jobs = state.jobs.write().await;
        if let Some(j) = jobs.get_mut(&job_id) {
            j.status = JobStatus::Running;
        }
    }

    match vm_lifecycle(&state, &job_id, &language, &code).await {
        Ok(agent_resp) => {
            let exit = agent_resp.exit_code;
            let mut jobs = state.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                j.status = JobStatus::Done;
                j.exit_code = Some(agent_resp.exit_code);
                j.stdout = Some(agent_resp.stdout);
                j.stderr = Some(agent_resp.stderr);
            }
            info!("Job {} completed (exit={})", job_id, exit);
        }
        Err(e) => {
            // `e` is already a String – no non-Send type in this arm.
            error!("Job {} failed: {}", job_id, e);
            let mut jobs = state.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                j.status = JobStatus::Error;
                j.stderr = Some(e);
            }
        }
    }
}

/// Full lifecycle of one VM for one job:
///
/// 1. Allocate an IP from the pool.
/// 2. Create a TAP interface and attach it to the bridge.
/// 3. Boot the VM with QEMU (kernel + initramfs that embeds the agent).
/// 4. Wait until the agent inside the VM responds to health checks.
/// 5. POST the code to the agent and collect the result.
/// 6. Kill the QEMU process, delete the TAP interface, release the IP.
///
/// Returns `Err(String)` (not `Box<dyn Error>`) so the result is `Send`
/// and can be safely stored after crossing a `tokio::spawn` boundary.
async fn vm_lifecycle(
    state: &Arc<AppState>,
    job_id: &str,
    language: &str,
    code: &str,
) -> Result<AgentExecuteResponse, String> {
    // ── 1. Allocate IP ───────────────────────────────────────────────
    let vm_ip = state.ip_manager.allocate_ip(job_id).map_err(|e| e.to_string())?;
    info!("Job {} – allocated VM IP {}", job_id, vm_ip);

    // TAP name: "tap-" + first 8 hex chars of the UUID → 12 chars (≤ 15)
    let tap_name = format!("tap-{}", &job_id.replace('-', "")[..8]);

    // ── 2. Create TAP and attach to bridge ───────────────────────────
    if let Err(e) = create_tap_device(&tap_name).await {
        let _ = state.ip_manager.release_ip(job_id);
        return Err(format!("TAP creation failed: {}", e));
    }
    if let Err(msg) = setup_guest_iface(&tap_name, &state.bridge_name)
        .await
        .map_err(|e| format!("TAP bridge attachment failed: {}", e))
    {
        cleanup_vm(&tap_name, job_id, &state.ip_manager).await;
        return Err(msg);
    }

    // ── 3. Démarrer la VM via le VMM du workspace ────────────────────
    let vm_ip_addr: Ipv4Addr = match vm_ip.parse() {
        Ok(a) => a,
        Err(e) => {
            cleanup_vm(&tap_name, job_id, &state.ip_manager).await;
            return Err(format!("Invalid VM IP '{}': {}", vm_ip, e));
        }
    };

    let (vm_handle, vm_stopper) = match start_vm(
        state.kernel_path.clone(),
        state.agent_initramfs.clone(),
        tap_name.clone(),
        vm_ip_addr,
        state.host_ip,
        state.netmask,
    )
    .await
    {
        Ok(r) => r,
        Err(msg) => {
            cleanup_vm(&tap_name, job_id, &state.ip_manager).await;
            return Err(msg);
        }
    };
    info!("Job {} – VM démarrée via VMM (TAP={})", job_id, tap_name);

    // ── 4. Attendre que l'agent soit prêt ───────────────────────────
    let agent_base = format!("http://{}:{}", vm_ip, state.agent_port);
    if let Err(msg) = poll_agent_health(&state.client, &agent_base).await {
        stop_vm(vm_stopper, vm_handle).await;
        cleanup_vm(&tap_name, job_id, &state.ip_manager).await;
        return Err(msg);
    }
    info!("Job {} – agent prêt à {}", job_id, agent_base);

    // ── 5. Envoyer le code à l'agent ────────────────────────────────
    let execute_result = state
        .client
        .post(format!("{}/execute", agent_base))
        .json(&AgentExecuteRequest {
            language: language.to_string(),
            code: code.to_string(),
        })
        .send()
        .await;

    // ── 6. Arrêter la VM + cleanup (toujours) ───────────────────────
    stop_vm(vm_stopper, vm_handle).await;
    cleanup_vm(&tap_name, job_id, &state.ip_manager).await;

    // Process result after cleanup so we don't leave orphan TAPs on error
    match execute_result {
        Ok(resp) if resp.status().is_success() => {
            let body: AgentExecuteResponse = resp.json().await.map_err(|e| e.to_string())?;
            Ok(body)
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(format!("Agent HTTP {}: {}", status, body))
        }
        Err(e) => Err(format!("Agent request failed: {}", e)),
    }
}

// ── Helpers: VM cleanup ───────────────────────────────────────────────

/// Best-effort cleanup for a finished job: delete the TAP interface and
/// release the IP back to the pool. Logs warnings on failure but never
/// surfaces errors to the caller.
async fn cleanup_vm(tap: &str, job_id: &str, ip_manager: &Arc<IpManager>) {
    if let Err(e) = delete_tap_device(tap).await {
        warn!("Could not delete TAP {}: {}", tap, e);
    }
    if let Err(e) = ip_manager.release_ip(job_id) {
        warn!("Could not release IP for job {}: {}", job_id, e);
    }
}

// ── Helpers: TAP device management ────────────────────────────────────

async fn create_tap_device(name: &str) -> Result<(), String> {
    let out = Command::new("ip")
        .args(["tuntap", "add", "dev", name, "mode", "tap"])
        .output()
        .await
        .map_err(|e| format!("ip tuntap add failed: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "ip tuntap add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

async fn delete_tap_device(name: &str) -> Result<(), String> {
    let out = Command::new("ip")
        .args(["link", "delete", name])
        .output()
        .await
        .map_err(|e| format!("ip link delete failed: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "ip link delete failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

// ── Helpers: VMM ─────────────────────────────────────────────────────

/// Crée un VMM (KVM direct) dans un thread dédié et renvoie :
/// - un `JoinHandle` pour attendre la fin du thread
/// - un `Arc<AtomicBool>` (stopper) pour ordonner l'arrêt de la VM
///
/// Le VMM est créé *à l'intérieur* du thread pour contourner
/// le fait que `VMM` n'implémente pas `Send` (trait objects internes).
/// La configuration (Send + 'static) est passée en paramètres.
async fn start_vm(
    kernel_path: String,
    initramfs_path: String,
    tap_name: String,
    vm_ip: Ipv4Addr,
    host_ip: Ipv4Addr,
    netmask: Ipv4Addr,
) -> Result<(std::thread::JoinHandle<()>, Arc<AtomicBool>), String> {
    // Canal one-shot : le thread envoie le stopper dès que le VMM est
    // configuré (avant de lancer vmm.run()) pour que l'appelant puisse
    // arrêter la VM depuis un autre thread.
    let (stopper_tx, stopper_rx) =
        tokio::sync::oneshot::channel::<Result<Arc<AtomicBool>, String>>();

    let handle = std::thread::Builder::new()
        .name(format!("vm-{}", tap_name))
        .spawn(move || {
            // Stdin : /dev/null (la VM n'est pas interactive)
            let stdin = match std::fs::File::open("/dev/null") {
                Ok(f) => f,
                Err(e) => {
                    let _ = stopper_tx.send(Err(format!("open /dev/null: {}", e)));
                    return;
                }
            };

            // Crée le VMM avec 256 Mio de RAM ; sortie série ignorée
            // (on utilise HTTP pour parler à l'agent).
            let mut vmm = match VMM::new(
                Box::new(stdin),
                Box::new(std::io::sink()),
                256 << 20, // 256 MiB
            ) {
                Ok(v) => v,
                Err(e) => {
                    let _ = stopper_tx.send(Err(format!("VMM::new: {:?}", e)));
                    return;
                }
            };

            // Attache la carte réseau VirtIO au TAP.
            // Le VMM génère automatiquement le paramètre ip= dans la cmdline.
            if let Err(e) =
                vmm.add_net_device(tap_name.clone(), Some(vm_ip), Some(host_ip), Some(netmask))
            {
                let _ = stopper_tx.send(Err(format!("add_net_device: {:?}", e)));
                return;
            }

            // Charge le kernel + initramfs en mémoire guest (2 vCPUs)
            if let Err(e) = vmm.configure(2, &kernel_path, &initramfs_path, None) {
                let _ = stopper_tx.send(Err(format!("configure: {:?}", e)));
                return;
            }

            // Expose le stopper AVANT vmm.run()
            let stopper = vmm.stopper();
            let _ = stopper_tx.send(Ok(stopper));
            // stopper_tx est détruit ici.

            // Démarre la VM ; bloque jusqu'à ce que le guest s'arrête
            // (VcpuExit::Shutdown | Hlt → running = false → run() retourne).
            vmm.run();
            info!("VM thread terminé (tap={})", tap_name);
        })
        .map_err(|e| format!("thread spawn failed: {}", e))?;

    // Attend la confirmation ou l'erreur d'init du VMM
    let stopper = stopper_rx
        .await
        .map_err(|_| "Le thread VMM s'est arrêté avant d'envoyer le stopper".to_string())?
        .map_err(|e| e)?;

    Ok((handle, stopper))
}

/// Signale à la VM de s'arrêter (stopper = false) puis attend la fin
/// du thread (timeout 10 s).
async fn stop_vm(stopper: Arc<AtomicBool>, handle: std::thread::JoinHandle<()>) {
    stopper.store(false, Ordering::SeqCst);
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            let _ = handle.join();
        }),
    )
    .await;
}

// ── Helpers: agent health polling ────────────────────────────────────

/// Retry GET `{agent_base}/health` every 2 s, for up to 60 attempts
/// (≈ 2 min).  Returns as soon as the agent answers 2xx.
async fn poll_agent_health(
    client: &reqwest::Client,
    agent_base: &str,
) -> Result<(), String> {
    let url = format!("{}/health", agent_base);
    const MAX_ATTEMPTS: u32 = 60;
    const INTERVAL: Duration = Duration::from_secs(2);

    for attempt in 1..=MAX_ATTEMPTS {
        tokio::time::sleep(INTERVAL).await;
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                warn!(
                    "Health check {}/{}: unexpected HTTP {}",
                    attempt,
                    MAX_ATTEMPTS,
                    resp.status()
                );
            }
            Err(_) => {
                // Normal during the early boot phase – keep retrying silently
            }
        }
    }

    Err(format!("Agent not healthy after {} attempts", MAX_ATTEMPTS))
}
