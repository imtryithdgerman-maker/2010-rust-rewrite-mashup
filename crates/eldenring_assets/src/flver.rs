//! FLVER2 models: skeleton, materials and triangle meshes.
//!
//! Reference: SoulsFormats `FLVER2.cs`, `FLVER2.Mesh.cs`, `FLVER2.FaceSet.cs`,
//! `FLVER2.VertexBuffer.cs`, `FLVER2.BufferLayout.cs`, `FLVER.LayoutMember.cs`.
//! Only the parts needed for collision/rendering are decoded; everything
//! else is skipped. Unknown vertex member types are ignored with a default.
use crate::error::{Error, Result};
use crate::reader::Bytes;

const HEADER_SIZE: usize = 0x80;
const DUMMY_SIZE: usize = 0x40;
const MATERIAL_SIZE: usize = 0x20;
const BONE_SIZE: usize = 0x80;
const MESH_SIZE: usize = 0x30;
const FACESET_SIZE: usize = 0x20;
const VERTEX_BUFFER_SIZE: usize = 0x20;
const LAYOUT_SIZE: usize = 0x10;
const MEMBER_SIZE: usize = 0x14;

/// Face-set flags marking LOD / motion-blur copies we skip.
const FACESET_SKIP_FLAGS: u32 = 0x0100_0000 | 0x0200_0000 | 0x8000_0000;

/// One bone ("node") of the skeleton, in parent space.
#[derive(Clone, Debug, PartialEq)]
pub struct FlverBone {
    pub name: String,
    pub parent: i16,
    pub translation: [f32; 3],
    /// Euler radians.
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlverMaterial {
    pub name: String,
    /// Material definition path (`.matxml` in Elden Ring).
    pub mtd: String,
}

/// A decoded mesh: triangle list plus per-vertex attributes. Attribute
/// vectors are either empty (not present) or `vertices.len()` long.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlverMesh {
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Triangle list (strips are converted).
    pub indices: Vec<u32>,
    pub bone_indices: Vec<[u8; 4]>,
    pub bone_weights: Vec<[f32; 4]>,
    pub material_index: i32,
    pub default_bone: i32,
    pub dynamic: bool,
}

impl FlverMesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flver {
    pub version: u32,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub bones: Vec<FlverBone>,
    pub materials: Vec<FlverMaterial>,
    pub meshes: Vec<FlverMesh>,
}

pub fn is_flver(data: &[u8]) -> bool {
    data.starts_with(b"FLVER\0")
}

#[derive(Clone, Copy)]
struct Member {
    struct_offset: usize,
    kind: u32,
    semantic: u32,
    index: i32,
}

/// Parses a (decompressed) FLVER2 file.
pub fn parse(data: &[u8]) -> Result<Flver> {
    let b = Bytes::new(data, "FLVER");
    b.expect_magic(0, b"FLVER\0")?;
    if b.slice(6, 2)? != b"L\0" {
        return Err(Error::format("FLVER", "big-endian FLVERs are not supported"));
    }
    let version = b.u32(0x08)?;
    // TODO: verify against game files (ER ships 0x2001A..0x2001C)
    if !(0x20000..0x30000).contains(&version) {
        return Err(Error::format("FLVER", format!("unsupported version {version:#x}")));
    }
    let data_offset = b.offset_i32(0x0C)?;
    let dummy_count = b.count_i32(0x14)?;
    let material_count = b.count_i32(0x18)?;
    let bone_count = b.count_i32(0x1C)?;
    let mesh_count = b.count_i32(0x20)?;
    let vb_count = b.count_i32(0x24)?;
    let bounds_min = b.vec3(0x28)?;
    let bounds_max = b.vec3(0x34)?;
    let header_index_size = u32::from(b.u8(0x48)?);
    let unicode = b.u8(0x49)? != 0;
    let faceset_count = b.count_i32(0x50)?;
    let layout_count = b.count_i32(0x54)?;

    let materials_at = HEADER_SIZE + dummy_count * DUMMY_SIZE;
    let bones_at = materials_at + material_count * MATERIAL_SIZE;
    let meshes_at = bones_at + bone_count * BONE_SIZE;
    let facesets_at = meshes_at + mesh_count * MESH_SIZE;
    let vbs_at = facesets_at + faceset_count * FACESET_SIZE;
    let layouts_at = vbs_at + vb_count * VERTEX_BUFFER_SIZE;
    b.slice(layouts_at, layout_count * LAYOUT_SIZE)?;

    let mut materials = Vec::with_capacity(material_count);
    for i in 0..material_count {
        let at = materials_at + i * MATERIAL_SIZE;
        materials.push(FlverMaterial {
            name: b.string(b.offset_i32(at)?, unicode).unwrap_or_else(|_| String::new()),
            mtd: b.string(b.offset_i32(at + 4)?, unicode).unwrap_or_else(|_| String::new()),
        });
    }

    let mut bones = Vec::with_capacity(bone_count);
    for i in 0..bone_count {
        let at = bones_at + i * BONE_SIZE;
        bones.push(FlverBone {
            translation: b.vec3(at)?,
            name: b.string(b.offset_i32(at + 0x0C)?, unicode).unwrap_or_else(|_| String::new()),
            rotation: b.vec3(at + 0x10)?,
            parent: b.i16(at + 0x1C)?,
            scale: b.vec3(at + 0x20)?,
        });
    }

    let layouts = (0..layout_count)
        .map(|i| {
            let at = layouts_at + i * LAYOUT_SIZE;
            let count = b.count_i32(at)?;
            let members_at = b.offset_i32(at + 0x0C)?;
            (0..count)
                .map(|m| {
                    let mo = members_at + m * MEMBER_SIZE;
                    Ok(Member {
                        struct_offset: b.offset_i32(mo + 4)?,
                        kind: b.u32(mo + 8)?,
                        semantic: b.u32(mo + 0x0C)?,
                        index: b.i32(mo + 0x10)?,
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;

    let mut meshes = Vec::with_capacity(mesh_count);
    for i in 0..mesh_count {
        let at = meshes_at + i * MESH_SIZE;
        let mut mesh = FlverMesh {
            dynamic: b.u8(at)? != 0,
            material_index: b.i32(at + 4)?,
            default_bone: b.i32(at + 0x10)?,
            ..FlverMesh::default()
        };
        let fs_count = b.count_i32(at + 0x20)?;
        let fs_indices_at = b.offset_i32(at + 0x24)?;
        let vb_count_mesh = b.count_i32(at + 0x28)?;
        let vb_indices_at = b.offset_i32(at + 0x2C)?;

        for v in 0..vb_count_mesh {
            let vb_index = b.count_i32(vb_indices_at + v * 4)?;
            if vb_index >= vb_count {
                return Err(Error::format("FLVER", format!("vertex buffer index {vb_index} out of range")));
            }
            let vb = vbs_at + vb_index * VERTEX_BUFFER_SIZE;
            let layout_index = b.count_i32(vb + 4)?;
            let vertex_size = b.count_i32(vb + 8)?;
            let vertex_count = b.count_i32(vb + 0x0C)?;
            let buffer_at = data_offset + b.offset_i32(vb + 0x1C)?;
            let layout = layouts
                .get(layout_index)
                .ok_or_else(|| Error::format("FLVER", format!("layout index {layout_index} out of range")))?;
            read_vertices(&b, &mut mesh, layout, buffer_at, vertex_size, vertex_count)?;
        }

        // Prefer the full-detail face set; fall back to the first one.
        let mut chosen: Option<(usize, u32)> = None;
        for f in 0..fs_count {
            let fs_index = b.count_i32(fs_indices_at + f * 4)?;
            if fs_index >= faceset_count {
                return Err(Error::format("FLVER", format!("face set index {fs_index} out of range")));
            }
            let flags = b.u32(facesets_at + fs_index * FACESET_SIZE)?;
            if chosen.is_none() || (flags & FACESET_SKIP_FLAGS == 0 && chosen.is_some_and(|(_, cf)| cf & FACESET_SKIP_FLAGS != 0)) {
                chosen = Some((fs_index, flags));
            }
        }
        if let Some((fs_index, _)) = chosen {
            let fs = facesets_at + fs_index * FACESET_SIZE;
            let strip = b.u8(fs + 4)? != 0;
            let index_count = b.count_i32(fs + 8)?;
            let indices_at = data_offset + b.offset_i32(fs + 0x0C)?;
            let mut index_size = if version >= 0x20009 { b.u32(fs + 0x18)? } else { 0 };
            if index_size == 0 {
                index_size = header_index_size;
            }
            let raw = read_indices(&b, indices_at, index_count, index_size)?;
            mesh.indices = if strip { triangulate_strip(&raw, index_size) } else { raw };
            let n = mesh.vertices.len() as u32;
            if mesh.indices.iter().any(|&ix| ix >= n) {
                return Err(Error::format("FLVER", format!("mesh {i}: vertex index out of range")));
            }
        }
        meshes.push(mesh);
    }

    Ok(Flver {
        version,
        bounds_min,
        bounds_max,
        bones,
        materials,
        meshes,
    })
}

fn read_indices(b: &Bytes<'_>, at: usize, count: usize, index_size: u32) -> Result<Vec<u32>> {
    match index_size {
        16 => (0..count).map(|i| b.u16(at + i * 2).map(u32::from)).collect(),
        32 => (0..count).map(|i| b.u32(at + i * 4)).collect(),
        // TODO: verify against game files (8-bit indices only appear in old games)
        8 => (0..count).map(|i| b.u8(at + i).map(u32::from)).collect(),
        other => Err(Error::format("FLVER", format!("unsupported index size {other}"))),
    }
}

/// Converts a triangle strip (with primitive restart) into a list,
/// keeping a consistent winding and dropping degenerate triangles.
pub fn triangulate_strip(strip: &[u32], index_size: u32) -> Vec<u32> {
    let restart = if index_size == 32 { u32::MAX } else { 0xFFFF };
    let mut out = Vec::new();
    let mut flip = false;
    for w in strip.windows(3) {
        let (a, bb, c) = (w[0], w[1], w[2]);
        if a == restart || bb == restart || c == restart {
            // Winding restarts after a restart marker.
            flip = false;
            continue;
        }
        if a != bb && bb != c && a != c {
            if flip {
                out.extend_from_slice(&[a, c, bb]);
            } else {
                out.extend_from_slice(&[a, bb, c]);
            }
        }
        flip = !flip;
    }
    out
}

fn snorm8(v: u8) -> f32 {
    (f32::from(v) - 127.0) / 127.0
}

fn read_vertices(
    b: &Bytes<'_>,
    mesh: &mut FlverMesh,
    layout: &[Member],
    buffer_at: usize,
    vertex_size: usize,
    vertex_count: usize,
) -> Result<()> {
    b.slice(buffer_at, vertex_size * vertex_count)?;
    let has = |sem: u32| layout.iter().any(|m| m.semantic == sem);
    // Several buffers may contribute to one mesh; only fill each attribute once.
    let fill_pos = has(0) && mesh.vertices.is_empty();
    let fill_nrm = has(3) && mesh.normals.is_empty();
    let fill_uv = has(5) && mesh.uvs.is_empty();
    let fill_bi = has(2) && mesh.bone_indices.is_empty();
    let fill_bw = has(1) && mesh.bone_weights.is_empty();

    for v in 0..vertex_count {
        let base = buffer_at + v * vertex_size;
        let mut uv_done = false;
        for m in layout {
            let at = base + m.struct_offset;
            match m.semantic {
                0 if fill_pos => mesh.vertices.push(read_vec3(b, at, m.kind)?),
                3 if fill_nrm => mesh.normals.push(read_vec3(b, at, m.kind)?),
                5 if fill_uv && !uv_done && m.index <= 0 => {
                    mesh.uvs.push(read_uv(b, at, m.kind)?);
                    uv_done = true;
                }
                2 if fill_bi => mesh.bone_indices.push(read_bone_indices(b, at, m.kind)?),
                1 if fill_bw => mesh.bone_weights.push(read_weights(b, at, m.kind)?),
                _ => {}
            }
        }
        if fill_uv && !uv_done {
            mesh.uvs.push([0.0; 2]);
        }
    }
    Ok(())
}

// Layout member types (SoulsFormats `FLVER.LayoutType`).
const FLOAT2: u32 = 0x01;
const FLOAT3: u32 = 0x02;
const FLOAT4: u32 = 0x03;
const BYTE4A: u32 = 0x10;
const BYTE4B: u32 = 0x11;
const SHORT2_TO_FLOAT2: u32 = 0x12;
const BYTE4C: u32 = 0x13;
const UV: u32 = 0x15;
const UV_PAIR: u32 = 0x16;
const SHORT_BONE_INDICES: u32 = 0x18;
const SHORT4_TO_FLOAT4A: u32 = 0x1A;
const SHORT4_TO_FLOAT4B: u32 = 0x2E;
const BYTE4E: u32 = 0x2F;

fn read_vec3(b: &Bytes<'_>, at: usize, kind: u32) -> Result<[f32; 3]> {
    Ok(match kind {
        FLOAT3 | FLOAT4 => b.vec3(at)?,
        BYTE4A | BYTE4B | BYTE4C | BYTE4E => [snorm8(b.u8(at)?), snorm8(b.u8(at + 1)?), snorm8(b.u8(at + 2)?)],
        // TODO: verify against game files (signed vs. biased shorts)
        SHORT4_TO_FLOAT4A => {
            let s = |o| b.i16(at + o).map(|v| f32::from(v) / 32767.0);
            [s(0)?, s(2)?, s(4)?]
        }
        SHORT4_TO_FLOAT4B => {
            let s = |o| b.u16(at + o).map(|v| (f32::from(v) - 32767.0) / 32767.0);
            [s(0)?, s(2)?, s(4)?]
        }
        _ => [0.0; 3],
    })
}

fn read_uv(b: &Bytes<'_>, at: usize, kind: u32) -> Result<[f32; 2]> {
    // TODO: verify against game files (ER UV scale assumed 1/2048)
    let short = |o| b.i16(at + o).map(|v| f32::from(v) / 2048.0);
    Ok(match kind {
        FLOAT2 | FLOAT3 | FLOAT4 => [b.f32(at)?, b.f32(at + 4)?],
        UV | UV_PAIR | SHORT2_TO_FLOAT2 => [short(0)?, short(2)?],
        _ => [0.0; 2],
    })
}

fn read_bone_indices(b: &Bytes<'_>, at: usize, kind: u32) -> Result<[u8; 4]> {
    Ok(match kind {
        SHORT_BONE_INDICES => {
            let s = |o| b.u16(at + o).map(|v| u8::try_from(v).unwrap_or(u8::MAX));
            [s(0)?, s(2)?, s(4)?, s(6)?]
        }
        _ => {
            let s = b.slice(at, 4)?;
            [s[0], s[1], s[2], s[3]]
        }
    })
}

fn read_weights(b: &Bytes<'_>, at: usize, kind: u32) -> Result<[f32; 4]> {
    Ok(match kind {
        SHORT4_TO_FLOAT4A => {
            let s = |o| b.i16(at + o).map(|v| f32::from(v) / 32767.0);
            [s(0)?, s(2)?, s(4)?, s(6)?]
        }
        FLOAT4 => [b.f32(at)?, b.f32(at + 4)?, b.f32(at + 8)?, b.f32(at + 12)?],
        _ => {
            let s = b.slice(at, 4)?;
            s.iter().map(|&v| f32::from(v) / 255.0).collect::<Vec<_>>().try_into().unwrap_or([0.0; 4])
        }
    })
}

/// Builds a minimal one-mesh FLVER2 (Float3 positions, Byte4 normals,
/// 16-bit triangle list). Test helper.
#[cfg(test)]
pub(crate) fn build_minimal(positions: &[[f32; 3]], indices: &[u16]) -> Vec<u8> {
    fn put_i32(v: &mut [u8], at: usize, x: i32) {
        v[at..at + 4].copy_from_slice(&x.to_le_bytes());
    }
    let meshes_at = HEADER_SIZE;
    let facesets_at = meshes_at + MESH_SIZE;
    let vbs_at = facesets_at + FACESET_SIZE;
    let layouts_at = vbs_at + VERTEX_BUFFER_SIZE;
    let members_at = layouts_at + LAYOUT_SIZE;
    let fs_index_at = members_at + 2 * MEMBER_SIZE;
    let vb_index_at = fs_index_at + 4;
    let data_offset = vb_index_at + 4;
    let vertex_size = 16;
    let vbytes = positions.len() * vertex_size;
    let total = data_offset + vbytes + indices.len() * 2;
    let mut v = vec![0u8; total];
    v[0..6].copy_from_slice(b"FLVER\0");
    v[6..8].copy_from_slice(b"L\0");
    put_i32(&mut v, 0x08, 0x2001A);
    put_i32(&mut v, 0x0C, data_offset as i32);
    put_i32(&mut v, 0x10, (total - data_offset) as i32);
    put_i32(&mut v, 0x20, 1); // meshes
    put_i32(&mut v, 0x24, 1); // vertex buffers
    v[0x48] = 16;
    put_i32(&mut v, 0x50, 1); // face sets
    put_i32(&mut v, 0x54, 1); // layouts
    // Mesh
    put_i32(&mut v, meshes_at + 0x20, 1);
    put_i32(&mut v, meshes_at + 0x24, fs_index_at as i32);
    put_i32(&mut v, meshes_at + 0x28, 1);
    put_i32(&mut v, meshes_at + 0x2C, vb_index_at as i32);
    // Face set
    put_i32(&mut v, facesets_at + 8, indices.len() as i32);
    put_i32(&mut v, facesets_at + 0x0C, vbytes as i32);
    put_i32(&mut v, facesets_at + 0x18, 16);
    // Vertex buffer
    put_i32(&mut v, vbs_at + 8, vertex_size as i32);
    put_i32(&mut v, vbs_at + 0x0C, positions.len() as i32);
    put_i32(&mut v, vbs_at + 0x18, vbytes as i32);
    put_i32(&mut v, vbs_at + 0x1C, 0);
    // Layout: position Float3 @0, normal Byte4A @12
    put_i32(&mut v, layouts_at, 2);
    put_i32(&mut v, layouts_at + 0x0C, members_at as i32);
    put_i32(&mut v, members_at + 8, FLOAT3 as i32);
    put_i32(&mut v, members_at + MEMBER_SIZE + 4, 12);
    put_i32(&mut v, members_at + MEMBER_SIZE + 8, BYTE4A as i32);
    put_i32(&mut v, members_at + MEMBER_SIZE + 0x0C, 3);
    for (i, p) in positions.iter().enumerate() {
        let at = data_offset + i * vertex_size;
        for (k, c) in p.iter().enumerate() {
            v[at + k * 4..at + k * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
        v[at + 12..at + 16].copy_from_slice(&[127, 254, 127, 0]);
    }
    for (i, ix) in indices.iter().enumerate() {
        let at = data_offset + vbytes + i * 2;
        v[at..at + 2].copy_from_slice(&ix.to_le_bytes());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_flver() {
        let data = build_minimal(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], &[0, 1, 2]);
        let flver = parse(&data).unwrap();
        assert_eq!(flver.meshes.len(), 1);
        let m = &flver.meshes[0];
        assert_eq!(m.vertices[1], [1.0, 0.0, 0.0]);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert_eq!(m.normals.len(), 3);
        assert!((m.normals[0][1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(b"NOTFLVER").is_err());
        let data = build_minimal(&[[0.0; 3]; 3], &[0, 1, 2]);
        assert!(parse(&data[..0x90]).is_err());
    }

    #[test]
    fn strip_conversion() {
        assert_eq!(triangulate_strip(&[0, 1, 2, 3], 16), vec![0, 1, 2, 1, 3, 2]);
        assert_eq!(triangulate_strip(&[0, 1, 2, 0xFFFF, 3, 4, 5], 16), vec![0, 1, 2, 3, 4, 5]);
    }
}
