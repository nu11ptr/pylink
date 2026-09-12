//! CI integration test application; run through scripts/smoke.py.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--print-home") {
        println!("{}", pylink::BUILD_PYTHON_HOME);
        return Ok(());
    }

    pylink::initialize()?;
    // Verify that repeated initialization is safe.
    pylink::initialize()?;
    println!(
        "Initialized Python {} from {}",
        pylink::PYTHON_VERSION,
        pylink::runtime_home()?.display()
    );
    Ok(())
}
