fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--print-home") {
        println!("{}", pybundle::BUILD_PYTHON_HOME);
        return Ok(());
    }

    pybundle::initialize()?;
    pybundle::initialize()?;
    println!(
        "Initialized Python {} from {}",
        pybundle::PYTHON_VERSION,
        pybundle::runtime_home()?.display()
    );
    Ok(())
}
