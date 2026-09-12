use pyo3::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pylink::initialize()?;

    let greeting = Python::attach(|py| -> PyResult<String> {
        py.eval(c"'Hello from Python!'", None, None)?.extract()
    })?;

    println!("{greeting}");

    Ok(())
}
