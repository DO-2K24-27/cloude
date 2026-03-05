use crate::ip_manager::IpManager;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{info, error, debug};

/// VM manager that coordinates with the agent
pub struct VmManager {
    // IP pool manager for allocating IPs to VMs
    ip_manager: Mutex<IpManager>,
    // HTTP client for communicating with the agent
    http_client: Client,
    // Agent server address
    agent_addr: String,
}

#[derive(Serialize)]
struct AgentExecuteRequest {
    language: String,
    code: String,
}

#[derive(Deserialize)]
struct AgentExecuteResponse {
    job_id: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

impl VmManager {
    pub fn new(ip_manager: IpManager) -> Self {
        let agent_addr = env::var("BACKEND_AGENT_ADDR")
            .unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());
        
        Self {
            ip_manager: Mutex::new(ip_manager),
            http_client: Client::new(),
            agent_addr,
        }
    }

    /// Allocates an IP address for a new VM
    /// 
    /// # Arguments
    /// * `vm_id` - Unique identifier for the VM
    /// 
    /// # Returns
    /// The allocated IP address as a String
    pub fn allocate_ip(&self, vm_id: &str) -> Result<String, String> {
        self.ip_manager
            .lock()
            .map_err(|e| format!("IP manager mutex poisoned: {}", e))?
            .allocate_ip(vm_id)
            .map_err(|e| format!("Failed to allocate IP: {}", e))
    }

    /// Releases the IP address associated with a VM
    /// 
    /// # Arguments
    /// * `vm_id` - Unique identifier for the VM
    pub fn release_ip(&self, vm_id: &str) -> Result<(), String> {
        self.ip_manager
            .lock()
            .map_err(|e| format!("IP manager mutex poisoned: {}", e))?
            .release_ip(vm_id)
            .map_err(|e| format!("Failed to release IP: {}", e))?;
        Ok(())
    }

    /// Sends code to the agent for execution
    /// 
    /// # Arguments
    /// * `vm_id` - Unique identifier for the VM (for tracking)
    /// * `vm_ip` - IP address assigned to this execution (for tracking)
    /// * `language` - Code language (python, node, rust)
    /// * `code` - Source code to execute
    pub async fn send_code_to_vm(
        &self,
        vm_id: &str,
        vm_ip: &str,
        language: &str,
        code: &str,
    ) -> Result<(), String> {
        info!(
            "Forwarding execution request to agent - VM ID: {}, IP: {}, Language: {}",
            vm_id, vm_ip, language
        );
        
        let agent_url = format!("{}/execute", self.agent_addr);
        
        let request = AgentExecuteRequest {
            language: language.to_string(),
            code: code.to_string(),
        };
        
        // Get timeout from environment or use default of 120 seconds
        let timeout_secs = env::var("BACKEND_AGENT_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(120);
        
        // Send request to agent with timeout
        let response = self.http_client
            .post(&agent_url)
            .json(&request)
            .timeout(Duration::from_secs(timeout_secs))
            .send()
            .await
            .map_err(|e| format!("Failed to send request to agent: {}", e))?;
        
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("Agent returned error {}: {}", status, error_text);
            return Err(format!("Agent execution failed: {} - {}", status, error_text));
        }
        
        let agent_response: AgentExecuteResponse = response.json()
            .await
            .map_err(|e| format!("Failed to parse agent response: {}", e))?;
        
        info!(
            "Agent completed execution - Job ID: {}, Exit code: {}",
            agent_response.job_id, agent_response.exit_code
        );
        
        // Log truncated output at debug level to avoid leaking secrets or huge payloads
        const MAX_OUTPUT_LEN: usize = 200;
        
        if !agent_response.stdout.is_empty() {
            let stdout_len = agent_response.stdout.len();
            if stdout_len <= MAX_OUTPUT_LEN {
                debug!("stdout ({} bytes): {}", stdout_len, agent_response.stdout);
            } else {
                let truncated = &agent_response.stdout[..MAX_OUTPUT_LEN];
                debug!("stdout ({} bytes, truncated): {}...(truncated)", stdout_len, truncated);
            }
        }
        
        if !agent_response.stderr.is_empty() {
            let stderr_len = agent_response.stderr.len();
            if stderr_len <= MAX_OUTPUT_LEN {
                debug!("stderr ({} bytes): {}", stderr_len, agent_response.stderr);
            } else {
                let truncated = &agent_response.stderr[..MAX_OUTPUT_LEN];
                debug!("stderr ({} bytes, truncated): {}...(truncated)", stderr_len, truncated);
            }
        }
        
        if agent_response.exit_code != 0 {
            return Err(format!(
                "Code execution failed with exit code {}",
                agent_response.exit_code
            ));
        }
        
        Ok(())
    }
}
