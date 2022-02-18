use super::Digest;

use pyo3::prelude::*;

#[derive(Debug, Clone)]
pub struct Manifest {
    pub size: Option<u64>,
    pub content_type: Option<String>,
    pub dependencies: Option<Vec<Digest>>,
}

impl FromPyObject<'_> for Manifest {
    fn extract(dict: &'_ PyAny) -> PyResult<Self> {
        // FIXME: This should send nice errors back to python if any of the unwraps fail...
        let size: u64 = dict.get_item("size").unwrap().extract().unwrap();
        let content_type: String = dict.get_item("content_type").unwrap().extract().unwrap();

        let dependencies = Vec::new();

        Ok(Manifest {
            size: Some(size),
            content_type: Some(content_type),
            dependencies: Some(dependencies),
        })
    }
}