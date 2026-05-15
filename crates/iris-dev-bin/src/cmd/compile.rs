use anyhow::{Context, Result};
use clap::Args;
use iris_dev_core::iris::{
    connection::{DiscoverySource, IrisConnection},
    discovery::{discover_iris, IrisDiscovery},
};

#[derive(Args)]
pub struct CompileCommand {
    pub target: Option<String>,
    #[arg(long, env = "IRIS_HOST")]
    pub host: Option<String>,
    #[arg(long, env = "IRIS_WEB_PORT", default_value = "52773")]
    pub web_port: u16,
    #[arg(long, env = "IRIS_NAMESPACE", default_value = "USER")]
    pub namespace: String,
    #[arg(long, env = "IRIS_USERNAME")]
    pub username: Option<String>,
    #[arg(long, env = "IRIS_PASSWORD")]
    pub password: Option<String>,
    #[arg(long, default_value = "cuk")]
    pub flags: String,
    #[arg(long)]
    pub force_writable: bool,
    #[arg(long, default_value = "text")]
    pub format: String,
}

impl CompileCommand {
    pub async fn run(self) -> Result<()> {
        let explicit = self.host.as_ref().map(|host| {
            let base_url = format!("http://{}:{}", host, self.web_port);
            let username = self.username.as_deref().unwrap_or("_SYSTEM");
            let password = self.password.as_deref().unwrap_or("SYS");
            IrisConnection::new(
                base_url,
                &self.namespace,
                username,
                password,
                DiscoverySource::ExplicitFlag,
            )
        });

        // Load .iris-dev.toml — takes precedence over env vars but not CLI flags (FR-006, FR-007).
        let ws_path = std::env::var("OBJECTSCRIPT_WORKSPACE").ok();
        let explicit = iris_dev_core::iris::workspace_config::apply_workspace_config(
            explicit,
            ws_path.as_deref(),
            &self.namespace,
        );

        let iris = match discover_iris(explicit).await {
            IrisDiscovery::Found(c) => c,
            IrisDiscovery::NotFound => {
                anyhow::bail!(
                    "No IRIS connection found — set IRIS_HOST or run iris-dev mcp for auto-discovery"
                );
            }
            IrisDiscovery::Explained => {
                // Specific actionable message already emitted to stderr — exit cleanly.
                std::process::exit(1);
            }
        };

        let client = IrisConnection::http_client()?;
        let target = self.target.as_deref().unwrap_or(".");

        let code = if target == "." {
            // Bug 1: CompileAll takes flags, not namespace. The namespace is selected by execute().
            format!(
                "Set sc=$SYSTEM.OBJ.CompileAll(\"{}\") If $System.Status.IsOK(sc) {{Write \"OK\"}} Else {{Write $System.Status.GetErrorText(sc)}}",
                self.flags
            )
        } else if target.ends_with(".cls") {
            let cls_text =
                std::fs::read_to_string(target).with_context(|| format!("reading {}", target))?;
            // Bug 2: derive class name from the "Class ..." declaration inside the file,
            // not from the file path (which would strip package components).
            let cls_name = cls_text
                .lines()
                .find(|l| l.trim_start().starts_with("Class "))
                .and_then(|l| l.split_whitespace().nth(1))
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    // Fallback: convert path separators to dots and strip extension
                    target
                        .trim_end_matches(".cls")
                        .replace(['/', '\\'], ".")
                        .trim_start_matches('.')
                        .to_string()
                });
            // FR-017/Mo3: use parameterized placeholders for both cls_name and cls_text.
            // Upload via Atelier PUT /doc/<name> then compile via ObjectScript.
            // This avoids the broken SELECT $SYSTEM.* approach (#53) and works
            // across all IRIS builds and web gateway configurations.
            let put_url = iris.versioned_ns_url(
                &self.namespace,
                &format!(
                    "/doc/{}?ignoreConflict=1",
                    urlencoding::encode(&format!("{}.cls", cls_name))
                ),
            );
            let lines: Vec<&str> = cls_text.lines().collect();
            let put_resp = client
                .put(&put_url)
                .basic_auth(&iris.username, Some(&iris.password))
                .json(&serde_json::json!({"enc": false, "content": lines}))
                .send()
                .await
                .context("PUT /doc failed")?;
            if !put_resp.status().is_success() {
                anyhow::bail!("Upload failed: HTTP {}", put_resp.status());
            }
            let put_body: serde_json::Value = put_resp.json().await.unwrap_or_default();
            if let Some(errs) = put_body["status"]["errors"].as_array() {
                if !errs.is_empty() {
                    let msg = errs[0]["error"].as_str().unwrap_or("Upload failed");
                    let result = serde_json::json!({"success": false, "error_code": "UPLOAD_FAILED", "error": msg, "target": target});
                    output_result(&result, &self.format);
                    std::process::exit(1);
                }
            }
            format!(
                "Set sc=$SYSTEM.OBJ.Compile(\"{}\",\"{}\") If $System.Status.IsOK(sc) {{Write \"OK\"}} Else {{Write $System.Status.GetErrorText(sc)}}",
                cls_name, self.flags
            )
        } else {
            format!(
                "Set sc=$SYSTEM.OBJ.Compile(\"{}\",\"{}\") If $System.Status.IsOK(sc) {{Write \"OK\"}} Else {{Write $System.Status.GetErrorText(sc)}}",
                target, self.flags
            )
        };

        // IDEV-1: try HTTP execution first (no IRIS_CONTAINER required).
        // Fall back to docker exec only if IRIS_CONTAINER is set to a non-empty value.
        let exec_result = match iris
            .execute_via_generator(&code, &self.namespace, &client)
            .await
        {
            Ok(out) => Ok(out),
            Err(_)
                if std::env::var("IRIS_CONTAINER")
                    .ok()
                    .filter(|v| !v.is_empty())
                    .is_some() =>
            {
                iris.execute(&code, &self.namespace).await
            }
            Err(e) => Err(e),
        };
        match exec_result {
            Ok(out) => {
                let out = out.trim().to_string();
                // execute_via_generator returns the ObjectScript Write output, which may
                // be prefixed by Atelier compile console lines (e.g. "Compilation started...
                // Compilation finished successfully in 0.000s.\nOK"). Check for "OK" at
                // the end, not exact equality.
                if out.ends_with("OK") || out == "OK" {
                    let result = serde_json::json!({"success": true, "target": target, "namespace": self.namespace, "stdout": "Compiled successfully"});
                    output_result(&result, &self.format);
                    Ok(())
                } else {
                    let result = serde_json::json!({"success": false, "error_code": "IRIS_COMPILE_FAILED", "error": out, "target": target});
                    output_result(&result, &self.format);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                let msg = e.to_string();
                let ec = if msg == "DOCKER_REQUIRED" {
                    "DOCKER_REQUIRED"
                } else {
                    "IRIS_UNREACHABLE"
                };
                let result = serde_json::json!({"success": false, "error_code": ec, "error": msg});
                output_result(&result, &self.format);
                std::process::exit(2);
            }
        }
    }
}

fn output_result(result: &serde_json::Value, format: &str) {
    if format == "json" {
        println!("{}", result);
    } else if result["success"] == true {
        println!("✓ Compiled: {}", result["target"].as_str().unwrap_or(""));
    } else {
        eprintln!(
            "✗ Error [{}]: {}",
            result["error_code"].as_str().unwrap_or(""),
            result["error"].as_str().unwrap_or("")
        );
    }
}
