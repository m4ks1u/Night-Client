//! Developer tool for UI testing without Microsoft sign-in: adds two demo
//! accounts to the database. "Alex" gets a dummy DPAPI-protected refresh
//! token (shown as signed in; renewal fails without a real token), "Steve"
//! gets none (shown as expired). The tokens are not valid credentials.
//!
//! ```text
//! cargo run -p nc-launcher --example seed_demo_accounts -- <db path> <vault dir>
//! cargo run -p nc-launcher --example seed_demo_accounts -- <db path> <vault dir> --remove
//! ```

#![allow(clippy::unwrap_used)]

use nc_core::Secret;
use nc_launcher::vault::{DpapiVault, TokenVault};

const ALEX: &str = "a1e0000000000000000000000000a1e0";
const STEVE: &str = "57e0000000000000000000000000057e";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (db, vault) = (
        args.first().expect("db path"),
        args.get(1).expect("vault dir"),
    );
    let store = nc_store::Store::open(std::path::Path::new(db)).unwrap();
    let vault = DpapiVault::new(vault);
    if args.iter().any(|a| a == "--remove") {
        for id in [ALEX, STEVE] {
            let _ = store.remove_account(id);
            vault.delete(id).unwrap();
        }
        println!("demo accounts removed");
        return;
    }
    store.upsert_active_account(STEVE, "DemoSteve").unwrap();
    store.upsert_active_account(ALEX, "DemoAlex").unwrap();
    vault
        .put(ALEX, &Secret::new("demo-not-a-real-refresh-token"))
        .unwrap();
    println!("seeded DemoAlex (active, token stored) and DemoSteve (no token)");
}
