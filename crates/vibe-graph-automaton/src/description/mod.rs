//! Description generation and inference modules.
//!
//! This module provides tools for:
//! - **Generation**: Static mapping from `SourceCodeGraph` to `AutomatonDescription`
//! - **Inference**: Hybrid learning using structural analysis + LLM interpretation

mod generator;

#[cfg(feature = "llm")]
mod inferencer;

pub use generator::{
    DescriptionGenerator, GeneratorConfig, NodeClassification, StabilityCalculator,
};

#[cfg(feature = "llm")]
pub use inferencer::{DescriptionInferencer, InferencerConfig, StructuralFeatures};
