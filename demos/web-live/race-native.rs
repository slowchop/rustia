fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seed = std::env::args()
        .nth(1)
        .ok_or("expected a race seed")?
        .parse()?;
    for distance in rustia_web_live_example::race(seed)? {
        println!("{distance}");
    }
    Ok(())
}
