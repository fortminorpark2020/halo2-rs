//! Shader (`shad`) tags: which bitmaps a surface uses.

use crate::mapset::MapSet;
use crate::{u32_at, DatumIndex, Result};

/// Runtime properties block: the engine's flattened view of the shader.
const SHAD_RUNTIME_PROPERTIES: usize = 12;
const RUNTIME_PROPERTIES_SIZE: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderInfo {
    /// Base colour texture, if the shader has one.
    pub diffuse: Option<DatumIndex>,
}

pub fn read_shader(set: &mut MapSet, shader: DatumIndex) -> Result<ShaderInfo> {
    let (src, _, data) = set.tag_data(shader)?;
    let file = set.get(src);
    let region = file.meta_region();
    let props = file.read_block(
        region,
        &data,
        SHAD_RUNTIME_PROPERTIES,
        RUNTIME_PROPERTIES_SIZE,
    )?;
    let diffuse = props
        .get(..RUNTIME_PROPERTIES_SIZE)
        .map(|p| DatumIndex(u32_at(p, 4)))
        .filter(|d| *d != DatumIndex::NONE);
    Ok(ShaderInfo { diffuse })
}
