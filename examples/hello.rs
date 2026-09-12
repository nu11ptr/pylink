//! Run with `cargo run --example hello`.

fn main() -> Result<(), pylink::Error> {
    pylink::initialize()?;

    println!(
        "Hello from Rust! Python {} is ready.",
        pylink::PYTHON_VERSION
    );

    Ok(())
}
