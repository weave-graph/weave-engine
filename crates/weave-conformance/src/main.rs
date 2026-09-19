fn main() {
    println!(
        "{}",
        String::from_utf8(weave_conformance::golden_profile()).unwrap()
    );
}
