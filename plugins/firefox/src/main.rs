use top_dog_firefox::{Firefox, marionette};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut plugin = Firefox::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--endpoint" => plugin.endpoints.push(marionette::endpoint(
                &args.next().ok_or("--endpoint needs IP:port")?,
            )?),
            "--include-private" => plugin.include_private = true,
            "--help" | "-h" => {
                println!(
                    "top-dog-firefox [--endpoint 127.0.0.1:2828]... [--include-private]\nServes protocol v1 JSON Lines on stdin/stdout. No endpoints are probed automatically."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    top_dog_protocol::serve(std::io::stdin().lock(), std::io::stdout().lock(), |op| {
        plugin.handle(op)
    })?;
    Ok(())
}
