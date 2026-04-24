use anyhow::Result;

use crate::exec;

/// Scans for WiFi networks and drops into connmanctl interactive mode for connection.
///
/// connmanctl is the connection manager available on Guix installer ISOs.
/// This is a thin wrapper — users can also run connmanctl directly.
pub fn wifi_connect() -> Result<()> {
    println!("Scanning for WiFi networks...");

    // Enable WiFi and scan
    let _ = exec::run_cmd(&["connmanctl", "enable", "wifi"]);
    exec::run_cmd(&["connmanctl", "scan", "wifi"])?;

    // List available services
    let result = exec::run_cmd(&["connmanctl", "services"])?;

    if result.stdout.trim().is_empty() {
        println!("No WiFi networks found.");
        return Ok(());
    }

    println!("\nAvailable networks:");
    println!("{}", result.stdout);

    // Prompt user for service name
    println!("To connect, use: connmanctl connect <service_id>");
    println!("Service IDs are shown in the right column above.");
    println!("For WPA networks, connmanctl will prompt for the passphrase.");

    // Try interactive connmanctl for connection
    println!("\nStarting connmanctl interactive mode...");
    println!("  Type: agent on");
    println!("  Then: connect <service_id>");
    println!("  Then: quit");
    exec::run_cmd_interactive(&["connmanctl"])?;

    Ok(())
}
