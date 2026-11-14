fn main() {
    println!("cargo:rerun-if-changed=../../dist");
    println!("cargo:rerun-if-changed=../../shared/device-names.json");
}
