fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&schemars::schema_for!(top_dog_protocol::Request)).unwrap()
    );
}
