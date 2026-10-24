fn main() {
    if let Err(error) =
        top_dog::cli::Options::parse(std::env::args().skip(1)).and_then(top_dog::cli::run)
    {
        eprintln!("top-dog: {error}");
        std::process::exit(1);
    }
}
