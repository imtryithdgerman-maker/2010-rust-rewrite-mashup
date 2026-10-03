//! Conversion of FLVER meshes and MSB placements into the engine's
//! XModel-style collision representation (`xmodel_runtime::CollTri`).
//!
//! `xmodel_runtime` has no owned `XModel` type (the IW4 `XModel` is a raw
//! pointer-offset POD), so this crate defines a small owned [`XModel`]
//! carrying render geometry plus collision surfaces in the exact layout
//! `clipmap_iw4::xmodel_trace` consumes.
use std::collections::HashMap;

use glam::{EulerRot, Mat4, Quat, Vec3};
use xmodel_runtime::{CollSurfCollision, CollTri};

use crate::flver::{Flver, FlverMesh};
use crate::msb::MapLayout;

/// IW-style "solid" contents bit used for world collision.
pub const CONTENTS_SOLID: u32 = 0x1;

/// An owned, engine-friendly model built from a FLVER.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XModel {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// One collision surface per source mesh.
    pub coll_surfs: Vec<CollSurfCollision>,
}

impl XModel {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Collision triangle for `p0, p1, p2`: plane `(n, d)` with `d = n·p0`, and
/// barycentric planes `svec`/`tvec` so that for a point `p` on the plane
/// `s = svec.xyz·p - svec.w`, `t = tvec.xyz·p - tvec.w` and the point lies
/// inside iff `s >= 0, t >= 0, s + t <= 1`. Returns `None` if degenerate.
pub fn coll_tri(p0: Vec3, p1: Vec3, p2: Vec3) -> Option<CollTri> {
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let n = e1.cross(e2);
    let len2 = n.length_squared();
    if len2 <= f32::EPSILON * f32::EPSILON || !len2.is_finite() {
        return None;
    }
    let unit = n / len2.sqrt();
    let s = e2.cross(n) / len2;
    let t = n.cross(e1) / len2;
    Some(CollTri {
        plane: [unit.x, unit.y, unit.z, unit.dot(p0)],
        svec: [s.x, s.y, s.z, s.dot(p0)],
        tvec: [t.x, t.y, t.z, t.dot(p0)],
    })
}

fn bounds(points: impl IntoIterator<Item = Vec3>) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for p in points {
        min = min.min(p);
        max = max.max(p);
    }
    if min.x > max.x { (Vec3::ZERO, Vec3::ZERO) } else { (min, max) }
}

fn mesh_coll_surf(mesh: &FlverMesh) -> CollSurfCollision {
    let v = |i: u32| Vec3::from(mesh.vertices[i as usize]);
    let tris: Vec<CollTri> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|t| coll_tri(v(t[0]), v(t[1]), v(t[2])))
        .collect();
    let (min, max) = bounds(mesh.indices.iter().map(|&i| v(i)));
    CollSurfCollision {
        bone: u16::try_from(mesh.default_bone.max(0)).unwrap_or(0),
        contents: CONTENTS_SOLID,
        surf_flags: 0,
        midpoint: ((min + max) * 0.5).to_array(),
        half_size: ((max - min) * 0.5).to_array(),
        tris,
    }
}

/// Converts one FLVER mesh into an [`XModel`].
pub fn flver_to_xmodel(mesh: &FlverMesh) -> XModel {
    flver_meshes_to_xmodel("", std::slice::from_ref(mesh))
}

/// Merges several meshes (e.g. all meshes of one FLVER) into one model.
pub fn flver_meshes_to_xmodel(name: &str, meshes: &[FlverMesh]) -> XModel {
    let mut model = XModel {
        name: name.to_owned(),
        ..XModel::default()
    };
    for mesh in meshes {
        let base = model.positions.len() as u32;
        let n = mesh.vertices.len();
        model.positions.extend_from_slice(&mesh.vertices);
        model.normals.extend((0..n).map(|i| mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0])));
        model.uvs.extend((0..n).map(|i| mesh.uvs.get(i).copied().unwrap_or([0.0; 2])));
        model.indices.extend(mesh.indices.iter().map(|&i| i + base));
        let surf = mesh_coll_surf(mesh);
        if !surf.tris.is_empty() {
            model.coll_surfs.push(surf);
        }
    }
    let (min, max) = bounds(model.positions.iter().map(|&p| Vec3::from(p)));
    model.bounds_min = min.to_array();
    model.bounds_max = max.to_array();
    model
}

/// Converts a whole FLVER into one [`XModel`].
pub fn flver_file_to_xmodel(name: &str, flver: &Flver) -> XModel {
    flver_meshes_to_xmodel(name, &flver.meshes)
}

/// World transform of an MSB placement: `T · R · S` with rotation in
/// degrees.
// TODO: verify against game files — FromSoftware applies X, then Z, then Y
// (row-vector `RotX * RotZ * RotY` in SoulsFormats tooling); glam is
// column-vector, hence `YZX` order here. Handedness may also need an X flip.
pub fn placement_matrix(position: [f32; 3], rotation_deg: [f32; 3], scale: [f32; 3]) -> Mat4 {
    let [rx, ry, rz] = rotation_deg.map(f32::to_radians);
    let rot = Quat::from_euler(EulerRot::YZX, ry, rz, rx);
    Mat4::from_scale_rotation_translation(Vec3::from(scale), rot, Vec3::from(position))
}

/// Pairs every map piece and asset with its model. Placements whose model
/// was not loaded are skipped.
pub fn msb_to_scene_with(layout: &MapLayout, models: &HashMap<String, XModel>) -> Vec<(XModel, Mat4)> {
    layout
        .map_pieces
        .iter()
        .chain(&layout.assets)
        .filter_map(|p| {
            let model = models.get(&p.model)?;
            Some((model.clone(), placement_matrix(p.position, p.rotation, p.scale)))
        })
        .collect()
}

/// Placeholder scene from a layout alone: each placement becomes an empty
/// named model with its transform (geometry is filled in by
/// [`crate::world_loader`], which has archive access).
pub fn msb_to_scene(layout: &MapLayout) -> Vec<(XModel, Mat4)> {
    layout
        .map_pieces
        .iter()
        .chain(&layout.assets)
        .map(|p| {
            let model = XModel {
                name: p.model.clone(),
                ..XModel::default()
            };
            (model, placement_matrix(p.position, p.rotation, p.scale))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msb::MapPiece;

    fn st(tri: &CollTri, p: Vec3) -> (f32, f32) {
        let d = |v: [f32; 4]| Vec3::new(v[0], v[1], v[2]).dot(p) - v[3];
        (d(tri.svec), d(tri.tvec))
    }

    #[test]
    fn coll_tri_barycentrics() {
        let (p0, p1, p2) = (Vec3::new(1.0, 2.0, 3.0), Vec3::new(3.0, 2.0, 3.0), Vec3::new(1.0, 2.0, 5.0));
        let tri = coll_tri(p0, p1, p2).unwrap();
        let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5;
        assert!(close(st(&tri, p0), (0.0, 0.0)));
        assert!(close(st(&tri, p1), (1.0, 0.0)));
        assert!(close(st(&tri, p2), (0.0, 1.0)));
        let (s, t) = st(&tri, Vec3::new(1.5, 2.0, 3.5));
        assert!(s > 0.0 && t > 0.0 && s + t < 1.0);
        // Plane distance of a point on the triangle is zero.
        let n = Vec3::new(tri.plane[0], tri.plane[1], tri.plane[2]);
        assert!((n.dot(p1) - tri.plane[3]).abs() < 1e-5);
        assert!(coll_tri(p0, p0, p1).is_none());
    }

    #[test]
    fn mesh_to_xmodel_and_scene() {
        let mesh = FlverMesh {
            vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            indices: vec![0, 1, 2],
            ..FlverMesh::default()
        };
        let model = flver_to_xmodel(&mesh);
        assert_eq!(model.coll_surfs.len(), 1);
        assert_eq!(model.coll_surfs[0].tris.len(), 1);
        assert_eq!(model.bounds_max, [1.0, 0.0, 1.0]);

        let layout = MapLayout {
            map_pieces: vec![MapPiece {
                name: "a".into(),
                model: "m000100".into(),
                position: [5.0, 0.0, 0.0],
                rotation: [0.0, 90.0, 0.0],
                scale: [1.0; 3],
            }],
            ..MapLayout::default()
        };
        let models = HashMap::from([("m000100".to_owned(), model)]);
        let scene = msb_to_scene_with(&layout, &models);
        assert_eq!(scene.len(), 1);
        let p = scene[0].1.transform_point3(Vec3::X);
        assert!((p - Vec3::new(5.0, 0.0, -1.0)).length() < 1e-5, "{p}");
        assert_eq!(msb_to_scene(&layout).len(), 1);
    }
}
