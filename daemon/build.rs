use std::env;
use std::fs;
use std::path::Path;

fn generate_sanctioned_addr_match(addr: &str) -> String {
    return format!(" \"{}\" ", addr);
}

fn generate_sanctioned_addr_vec_entry(addr: &str) -> String {
    return format!("\"{}\".to_string(),\n", addr);
}

fn main() {
    let ofac_addr_json = fs::read_to_string("../OFAC_sanctioned_addresses.json").unwrap();

    let ofac_sanctioned_addresses: Vec<String> = serde_json::from_str(&ofac_addr_json).unwrap();

    let out_dir = env::var_os("OUT_DIR").unwrap();
    let match_dest_path = Path::new(&out_dir).join("match_sanctioned_addr.rs");
    let list_dest_path = Path::new(&out_dir).join("list_sanctioned_addr.rs");

    let ofac_addr_match = ofac_sanctioned_addresses
        .iter()
        .map(|addr| generate_sanctioned_addr_match(addr))
        .collect::<Vec<String>>()
        .join("|");

    let mut ofac_addr_list = String::default();
    for addr in ofac_sanctioned_addresses.iter() {
        ofac_addr_list += &generate_sanctioned_addr_vec_entry(addr);
    }

    fs::write(
        &match_dest_path,
        format!(
            "// DON'T CHANGE THIS FILE MANUALLY. IT WILL BE OVERWRITTEN.
// This is an automatically generated file.
// Change it's generation in build.rs.

#[allow(non_snake_case)]
fn is_OFAC_sanctioned(addr: &Address) -> bool {{
    matches!(addr.to_string().as_str(), {})
}}

",
            ofac_addr_match
        ),
    )
    .unwrap();

    fs::write(
        &list_dest_path,
        format!(
            "// DON'T CHANGE THIS FILE MANUALLY. IT WILL BE OVERWRITTEN.
// This is an automatically generated file.
// Change it's generation in build.rs.

#[allow(non_snake_case)]
fn get_OFAC_sanctioned_addresses() -> Vec<String> {{
    vec![
        {}
    ]
}}

",
            ofac_addr_list
        ),
    )
    .unwrap();

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=OFAC_sanctioned_addresses.json");
}
