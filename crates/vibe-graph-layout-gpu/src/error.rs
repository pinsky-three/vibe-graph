//! Error types for GPU layout operations.

use thiserror::Error;

/// Errors that can occur during GPU layout operations.
#[derive(Error, Debug)]
pub enum LayoutError {
    /// Failed to initialize GPU device.
    #[error("GPU initialization failed: {0}")]
    GpuInit(String),

    /// Failed to create GPU resources.
    #[error("GPU resource creation failed: {0}")]
    ResourceCreation(String),

    /// Failed to execute GPU compute.
    #[error("GPU compute execution failed: {0}")]
    ComputeExecution(String),

    /// Failed to read back data from GPU.
    #[error("GPU readback failed: {0}")]
    Readback(String),

    /// Invalid graph data.
    #[error("Invalid graph: {0}")]
    InvalidGraph(String),

    /// Layout not initialized.
    #[error("Layout not initialized")]
    NotInitialized,
}

