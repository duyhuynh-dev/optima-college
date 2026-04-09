fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "../contracts/proto/optima/v1/kernel.proto";
    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(&[proto], &["../contracts/proto"])?;
    Ok(())
}
