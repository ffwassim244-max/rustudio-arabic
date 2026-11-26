//! 3D mesh export (Wavefront OBJ + MTL, glTF 2.0 / GLB) and minimal glTF 2.0 /
//! GLB mesh import, with zero extra crate dependencies.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use glam::{Mat3, Mat4, Quat, Vec2, Vec3};
use rbx_dom_weak::types::Ref;
use rustudio_core::doc::Document;
use rustudio_core::mesh::{MeshAsset, MeshData};
use rustudio_render::render::MeshKind;
use rustudio_render::scene::Scene;

/// One world-space part mesh ready to write to OBJ or glTF.
pub struct ExportPart {
    pub name: String,
    /// World-space triangle list (3 vertices per triangle).
    pub positions: Vec<Vec3>,
    /// World-space unit normals (same length as `positions`).
    pub normals: Vec<Vec3>,
    /// Texture coordinates (same length as `positions`; top-left origin).
    pub uvs: Vec<Vec2>,
    /// Linear RGB colour (`PartDraw.color`).
    pub color: [f32; 3],
    /// Opacity in `0.0..=1.0`.
    pub alpha: f32,
}

/// One mesh decoded from a `.glb` or `.gltf` file.
pub struct ImportedMesh {
    pub name: String,
    /// Triangle list (3 vertices per triangle).
    pub positions: Vec<Vec3>,
    /// Unit normals (same length as `positions`).
    pub normals: Vec<Vec3>,
    /// Texture coordinates (same length as `positions`; top-left origin).
    pub uvs: Vec<Vec2>,
    /// Optional sRGB base colour in `0.0..=1.0`.
    pub color: Option<[f32; 3]>,
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Collects world-transformed geometry for all `BasePart`s under `roots`
/// (including `roots` themselves and their descendants), or every 3D world
/// part in `scene` when `roots` is empty.
pub fn collect_parts(doc: &Document, scene: &Scene, roots: &[Ref]) -> Vec<ExportPart> {
    let mut filter: HashSet<Ref> = HashSet::new();
    if !roots.is_empty() {
        for &r in roots {
            filter.insert(r);
            for d in doc.dom.descendants_of(r) {
                filter.insert(d.referent());
            }
        }
    }
    let doc_dir = doc.path.as_deref().and_then(Path::parent);
    let mut out = Vec::new();
    for sp in &scene.parts {
        if sp.layer.is_some() {
            continue;
        }
        if !filter.is_empty() && !filter.contains(&sp.r) {
            continue;
        }
        let name = doc
            .dom
            .get_by_ref(sp.r)
            .map(|i| i.name.clone())
            .unwrap_or_else(|| "Part".into());
        let xf = sp.draw.xf;
        let size = sp.draw.size;
        let (positions, normals, uvs) = if let Some(asset) = &sp.mesh_asset {
            if let Some(m) = load_cached_mesh(asset, doc_dir) {
                transform_mesh_data(&m, asset, xf.pos, xf.rot, size)
            } else {
                primitive_world(sp.draw.mesh, xf.pos, xf.rot, size)
            }
        } else {
            primitive_world(sp.draw.mesh, xf.pos, xf.rot, size)
        };
        if positions.is_empty() {
            continue;
        }
        out.push(ExportPart {
            name,
            positions,
            normals,
            uvs,
            color: sp.draw.color,
            alpha: sp.draw.alpha.clamp(0.0, 1.0),
        });
    }
    out
}

fn load_cached_mesh(asset: &MeshAsset, doc_dir: Option<&Path>) -> Option<MeshData> {
    let id = asset.id.as_str();
    let bytes: Vec<u8> = rustudio_core::assets::generated(id)
        .map(|b| b.to_vec())
        .or_else(|| match rustudio_core::assets::asset_ref(id) {
            Some(a) => rustudio_core::assets::local_asset_path(a.id)
                .and_then(|p| std::fs::read(p).ok())
                .or_else(|| std::fs::read(rustudio_core::assets::cache_path(a)).ok()),
            None => rustudio_core::assets::resolve(id, doc_dir).and_then(|p| std::fs::read(p).ok()),
        })?;
    let m = rustudio_core::mesh::parse(&bytes).ok()?;
    if m.positions.len() < 3 {
        return None;
    }
    Some(m)
}

fn transform_mesh_data(
    m: &MeshData,
    asset: &MeshAsset,
    pos: Vec3,
    rot: Mat3,
    size: Vec3,
) -> (Vec<Vec3>, Vec<Vec3>, Vec<Vec2>) {
    let n = m.positions.len() - (m.positions.len() % 3);
    let mut positions = Vec::with_capacity(n);
    let mut normals = Vec::with_capacity(n);
    let mut uvs = Vec::with_capacity(n);
    let ext = (m.max - m.min).max(Vec3::splat(1e-4));
    let center = if asset.origin { Vec3::ZERO } else { (m.max + m.min) * 0.5 };
    let scale_vec = if asset.fit_to_part {
        size / ext
    } else {
        asset.scale
    };
    let inv_scale = Vec3::new(
        if scale_vec.x.abs() > 1e-6 { 1.0 / scale_vec.x } else { 1.0 },
        if scale_vec.y.abs() > 1e-6 { 1.0 / scale_vec.y } else { 1.0 },
        if scale_vec.z.abs() > 1e-6 { 1.0 / scale_vec.z } else { 1.0 },
    );
    for i in 0..n {
        let lp = if asset.fit_to_part {
            (m.positions[i] - center) * scale_vec
        } else {
            (m.positions[i] - center) * asset.scale + asset.offset
        };
        positions.push(pos + rot * lp);
        let ln = m.normals.get(i).copied().unwrap_or(Vec3::Y);
        normals.push((rot * (ln * inv_scale)).normalize_or_zero());
        uvs.push(m.uvs.get(i).copied().unwrap_or(Vec2::ZERO));
    }
    (positions, normals, uvs)
}

fn primitive_world(
    kind: MeshKind,
    pos: Vec3,
    rot: Mat3,
    size: Vec3,
) -> (Vec<Vec3>, Vec<Vec3>, Vec<Vec2>) {
    let (lp, ln, luv) = primitive_unit(kind);
    let inv_size = Vec3::new(
        if size.x.abs() > 1e-6 { 1.0 / size.x } else { 1.0 },
        if size.y.abs() > 1e-6 { 1.0 / size.y } else { 1.0 },
        if size.z.abs() > 1e-6 { 1.0 / size.z } else { 1.0 },
    );
    let positions = lp.into_iter().map(|p| pos + rot * (p * size)).collect();
    let normals = ln
        .into_iter()
        .map(|n| (rot * (n * inv_size)).normalize_or_zero())
        .collect();
    (positions, normals, luv)
}

#[derive(Default)]
struct PrimBuilder {
    pos: Vec<Vec3>,
    norm: Vec<Vec3>,
    uv: Vec<Vec2>,
    center: Vec3,
}

impl PrimBuilder {
    fn tri(&mut self, a: (Vec3, Vec3, Vec2), b: (Vec3, Vec3, Vec2), c: (Vec3, Vec3, Vec2)) {
        self.pos.extend_from_slice(&[a.0, b.0, c.0]);
        self.norm.extend_from_slice(&[a.1, b.1, c.1]);
        self.uv.extend_from_slice(&[a.2, b.2, c.2]);
    }

    fn flat(&mut self, pts: &[Vec3]) {
        if pts.len() < 3 {
            return;
        }
        let mut n = (pts[1] - pts[0]).cross(pts[2] - pts[0]).normalize_or_zero();
        let mid: Vec3 = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
        let flip = n.dot(mid - self.center) < 0.0;
        if flip {
            n = -n;
        }
        let uvs = [
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 0.0),
        ];
        for i in 1..pts.len() - 1 {
            let (p0, p1, p2) = if flip {
                (pts[0], pts[i + 1], pts[i])
            } else {
                (pts[0], pts[i], pts[i + 1])
            };
            let (u0, u1, u2) = if flip {
                (uvs[0], uvs[(i + 1).min(3)], uvs[i.min(3)])
            } else {
                (uvs[0], uvs[i.min(3)], uvs[(i + 1).min(3)])
            };
            self.tri((p0, n, u0), (p1, n, u1), (p2, n, u2));
        }
    }
}

fn primitive_unit(kind: MeshKind) -> (Vec<Vec3>, Vec<Vec3>, Vec<Vec2>) {
    let h = 0.5f32;
    let v = Vec3::new;
    let mut b = PrimBuilder::default();
    match kind {
        MeshKind::Block => {
            for axis in 0..3 {
                for s in [-h, h] {
                    let corner = |a: f32, c: f32| {
                        let mut p = Vec3::ZERO;
                        p[axis] = s;
                        p[(axis + 1) % 3] = a;
                        p[(axis + 2) % 3] = c;
                        p
                    };
                    b.flat(&[
                        corner(-h, -h),
                        corner(h, -h),
                        corner(h, h),
                        corner(-h, h),
                    ]);
                }
            }
        }
        MeshKind::Ball => {
            let (rings, segs) = (10usize, 24usize);
            let sp = |i: usize, j: usize| {
                let th = std::f32::consts::PI * i as f32 / rings as f32;
                let ph = std::f32::consts::TAU * j as f32 / segs as f32;
                let n = Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin());
                let uv = Vec2::new(j as f32 / segs as f32, i as f32 / rings as f32);
                (n * h, n, uv)
            };
            for i in 0..rings {
                for j in 0..segs {
                    let (p0, p1, p2, p3) = (sp(i, j), sp(i + 1, j), sp(i + 1, j + 1), sp(i, j + 1));
                    if i != 0 {
                        b.tri(p0, p1, p3);
                    }
                    if i != rings - 1 {
                        b.tri(p1, p2, p3);
                    }
                }
            }
        }
        MeshKind::Cylinder => {
            let segs = 24usize;
            let ring = |j: usize| {
                let a = std::f32::consts::TAU * j as f32 / segs as f32;
                Vec3::new(0.0, a.cos(), a.sin())
            };
            for j in 0..segs {
                let (n0, n1) = (ring(j), ring(j + 1));
                let (p0, p1) = (n0 * h, n1 * h);
                let u0 = j as f32 / segs as f32;
                let u1 = (j + 1) as f32 / segs as f32;
                b.tri(
                    (p0 + v(-h, 0.0, 0.0), n0, Vec2::new(u0, 0.0)),
                    (p0 + v(h, 0.0, 0.0), n0, Vec2::new(u0, 1.0)),
                    (p1 + v(h, 0.0, 0.0), n1, Vec2::new(u1, 1.0)),
                );
                b.tri(
                    (p0 + v(-h, 0.0, 0.0), n0, Vec2::new(u0, 0.0)),
                    (p1 + v(h, 0.0, 0.0), n1, Vec2::new(u1, 1.0)),
                    (p1 + v(-h, 0.0, 0.0), n1, Vec2::new(u1, 0.0)),
                );
                for s in [-h, h] {
                    let n = v(s.signum(), 0.0, 0.0);
                    let c_uv = Vec2::new(0.5, 0.5);
                    let uv0 = Vec2::new(0.5 + n0.y * 0.5, 0.5 + n0.z * 0.5);
                    let uv1 = Vec2::new(0.5 + n1.y * 0.5, 0.5 + n1.z * 0.5);
                    if s > 0.0 {
                        b.tri(
                            (v(s, 0.0, 0.0), n, c_uv),
                            (p0 + v(s, 0.0, 0.0), n, uv0),
                            (p1 + v(s, 0.0, 0.0), n, uv1),
                        );
                    } else {
                        b.tri(
                            (v(s, 0.0, 0.0), n, c_uv),
                            (p1 + v(s, 0.0, 0.0), n, uv1),
                            (p0 + v(s, 0.0, 0.0), n, uv0),
                        );
                    }
                }
            }
        }
        MeshKind::Wedge => {
            b.center = v(0.0, -0.2, 0.2);
            b.flat(&[v(-h, -h, -h), v(h, -h, -h), v(h, -h, h), v(-h, -h, h)]);
            b.flat(&[v(-h, -h, h), v(h, -h, h), v(h, h, h), v(-h, h, h)]);
            b.flat(&[v(-h, -h, -h), v(h, -h, -h), v(h, h, h), v(-h, h, h)]);
            b.flat(&[v(-h, -h, -h), v(-h, -h, h), v(-h, h, h)]);
            b.flat(&[v(h, -h, -h), v(h, -h, h), v(h, h, h)]);
        }
        MeshKind::CornerWedge => {
            b.center = v(0.25, -0.3, -0.25);
            let apex = v(h, h, -h);
            let (b0, b1, b2, b3) = (v(-h, -h, -h), v(h, -h, -h), v(h, -h, h), v(-h, -h, h));
            b.flat(&[b0, b1, b2, b3]);
            b.flat(&[apex, b1, b2]);
            b.flat(&[apex, b0, b1]);
            b.flat(&[apex, b2, b3]);
            b.flat(&[apex, b3, b0]);
        }
    }
    (b.pos, b.norm, b.uv)
}

fn safe_obj_name(s: &str, idx: usize) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
        format!("Part_{idx}")
    } else {
        format!("{cleaned}_{idx}")
    }
}

/// Writes Wavefront `.obj` and a sibling `.mtl` file.
pub fn write_obj(path: &Path, parts: &[ExportPart]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    let mtl_path = path.with_extension("mtl");
    let mtl_name = mtl_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("materials.mtl");

    let mut obj = String::with_capacity(parts.len() * 512);
    let mut mtl = String::with_capacity(parts.len() * 128);
    let _ = writeln!(obj, "# Exported by rustudio");
    let _ = writeln!(obj, "mtllib {mtl_name}");
    let _ = writeln!(mtl, "# Materials exported by rustudio");

    let mut base_idx = 1usize;
    for (i, part) in parts.iter().enumerate() {
        let count = part.positions.len() - (part.positions.len() % 3);
        if count == 0 {
            continue;
        }
        let obj_name = safe_obj_name(&part.name, i);
        let mat_name = format!("mat_{i}");
        let _ = writeln!(obj, "\no {obj_name}");
        let _ = writeln!(obj, "usemtl {mat_name}");
        for k in 0..count {
            let p = part.positions[k];
            let _ = writeln!(obj, "v {:.6} {:.6} {:.6}", p.x, p.y, p.z);
        }
        for k in 0..count {
            let n = part.normals.get(k).copied().unwrap_or(Vec3::Y);
            let _ = writeln!(obj, "vn {:.6} {:.6} {:.6}", n.x, n.y, n.z);
        }
        for k in 0..count {
            let uv = part.uvs.get(k).copied().unwrap_or(Vec2::ZERO);
            let _ = writeln!(obj, "vt {:.6} {:.6}", uv.x, 1.0 - uv.y);
        }
        for t in (0..count).step_by(3) {
            let a = base_idx + t;
            let b = a + 1;
            let c = a + 2;
            let _ = writeln!(obj, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}");
        }
        base_idx += count;

        let r = linear_to_srgb(part.color[0]);
        let g = linear_to_srgb(part.color[1]);
        let bl = linear_to_srgb(part.color[2]);
        let _ = writeln!(mtl, "\nnewmtl {mat_name}");
        let _ = writeln!(mtl, "Kd {r:.5} {g:.5} {bl:.5}");
        let _ = writeln!(mtl, "d {:.5}", part.alpha.clamp(0.0, 1.0));
    }

    std::fs::write(path, obj).map_err(|e| format!("write {}: {e}", path.display()))?;
    std::fs::write(&mtl_path, mtl).map_err(|e| format!("write {}: {e}", mtl_path.display()))?;
    Ok(())
}

/// Writes self-contained glTF 2.0 `.glb` (or `.gltf` + sibling `.bin` when
/// `path` ends with `.gltf`).
pub fn write_gltf(path: &Path, parts: &[ExportPart]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }

    let mut bin: Vec<u8> = Vec::new();
    let mut buffer_views = Vec::<serde_json::Value>::new();
    let mut accessors = Vec::<serde_json::Value>::new();
    let mut materials = Vec::<serde_json::Value>::new();
    let mut meshes = Vec::<serde_json::Value>::new();
    let mut nodes = Vec::<serde_json::Value>::new();

    for part in parts {
        let count = part.positions.len() - (part.positions.len() % 3);
        if count == 0 {
            continue;
        }
        // Align to 4 bytes before each view.
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let pos_offset = bin.len();
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for &p in &part.positions[..count] {
            lo = lo.min(p);
            hi = hi.max(p);
            for f in [p.x, p.y, p.z] {
                bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        let pos_len = bin.len() - pos_offset;
        let pos_bv = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": pos_offset,
            "byteLength": pos_len,
            "target": 34962
        }));
        let pos_acc = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": pos_bv,
            "componentType": 5126,
            "count": count,
            "type": "VEC3",
            "min": [lo.x, lo.y, lo.z],
            "max": [hi.x, hi.y, hi.z]
        }));

        let norm_offset = bin.len();
        for k in 0..count {
            let n = part.normals.get(k).copied().unwrap_or(Vec3::Y);
            for f in [n.x, n.y, n.z] {
                bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        let norm_len = bin.len() - norm_offset;
        let norm_bv = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": norm_offset,
            "byteLength": norm_len,
            "target": 34962
        }));
        let norm_acc = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": norm_bv,
            "componentType": 5126,
            "count": count,
            "type": "VEC3"
        }));

        let uv_offset = bin.len();
        for k in 0..count {
            let uv = part.uvs.get(k).copied().unwrap_or(Vec2::ZERO);
            for f in [uv.x, uv.y] {
                bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        let uv_len = bin.len() - uv_offset;
        let uv_bv = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": uv_offset,
            "byteLength": uv_len,
            "target": 34962
        }));
        let uv_acc = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": uv_bv,
            "componentType": 5126,
            "count": count,
            "type": "VEC2"
        }));

        let alpha = part.alpha.clamp(0.0, 1.0);
        let mat_idx = materials.len();
        materials.push(serde_json::json!({
            "name": format!("mat_{mat_idx}"),
            "pbrMetallicRoughness": {
                "baseColorFactor": [
                    part.color[0].clamp(0.0, 1.0),
                    part.color[1].clamp(0.0, 1.0),
                    part.color[2].clamp(0.0, 1.0),
                    alpha
                ],
                "metallicFactor": 0.0,
                "roughnessFactor": 0.6
            },
            "alphaMode": if alpha < 0.999 { "BLEND" } else { "OPAQUE" }
        }));

        let mesh_idx = meshes.len();
        meshes.push(serde_json::json!({
            "name": part.name,
            "primitives": [{
                "attributes": {
                    "POSITION": pos_acc,
                    "NORMAL": norm_acc,
                    "TEXCOORD_0": uv_acc
                },
                "material": mat_idx,
                "mode": 4
            }]
        }));
        nodes.push(serde_json::json!({
            "name": part.name,
            "mesh": mesh_idx
        }));
    }

    let node_indices: Vec<usize> = (0..nodes.len()).collect();
    let is_json_gltf = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gltf"));

    if is_json_gltf {
        let bin_path = path.with_extension("bin");
        let bin_name = bin_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("scene.bin");
        let doc = serde_json::json!({
            "asset": { "version": "2.0", "generator": "rustudio" },
            "scene": 0,
            "scenes": [{ "nodes": node_indices }],
            "nodes": nodes,
            "meshes": meshes,
            "materials": materials,
            "accessors": accessors,
            "bufferViews": buffer_views,
            "buffers": [{ "byteLength": bin.len(), "uri": bin_name }]
        });
        let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        std::fs::write(&bin_path, &bin).map_err(|e| format!("write {}: {e}", bin_path.display()))?;
        std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        return Ok(());
    }

    // Self-contained GLB (12-byte header + JSON chunk + BIN chunk).
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let doc = serde_json::json!({
        "asset": { "version": "2.0", "generator": "rustudio" },
        "scene": 0,
        "scenes": [{ "nodes": node_indices }],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": buffer_views,
        "buffers": [{ "byteLength": bin.len() }]
    });
    let mut json_bytes = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    let total_len = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total_len);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_len as u32).to_le_bytes());
    // JSON chunk
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    glb.extend_from_slice(&json_bytes);
    // BIN chunk
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    glb.extend_from_slice(&bin);

    std::fs::write(path, glb).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

/// Encodes de-indexed triangles into Roblox "version 2.00" binary mesh bytes,
/// ready for `rustudio_core::assets::register_generated`.
pub fn mesh_data_to_v2(positions: &[Vec3], normals: &[Vec3], uvs: &[Vec2]) -> Vec<u8> {
    let n = positions.len() - (positions.len() % 3);
    let mut b = Vec::with_capacity(32 + n * 36 + (n / 3) * 12);
    b.extend_from_slice(b"version 2.00\n");
    b.extend_from_slice(&12u16.to_le_bytes());
    b.push(36);
    b.push(12);
    b.extend_from_slice(&(n as u32).to_le_bytes());
    b.extend_from_slice(&((n / 3) as u32).to_le_bytes());
    for i in 0..n {
        let p = positions[i];
        let nn = normals.get(i).copied().unwrap_or(Vec3::Y);
        let uv = uvs.get(i).copied().unwrap_or(Vec2::ZERO);
        for f in [p.x, p.y, p.z, nn.x, nn.y, nn.z, uv.x, uv.y, 0.0] {
            b.extend_from_slice(&f.to_le_bytes());
        }
    }
    for t in 0..n / 3 {
        for k in 0..3 {
            b.extend_from_slice(&((t * 3 + k) as u32).to_le_bytes());
        }
    }
    b
}

/// Parses a glTF 2.0 `.glb` or `.gltf` file into world-transformed meshes.
pub fn parse_gltf(path: &Path, bytes: &[u8]) -> Result<Vec<ImportedMesh>, String> {
    let (json, bin_chunk): (serde_json::Value, Option<&[u8]>) =
        if bytes.len() >= 12 && &bytes[0..4] == b"glTF" {
            let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            if version != 2 {
                return Err(format!("Unsupported GLB version {version}"));
            }
            let mut offset = 12usize;
            let mut json_val: Option<serde_json::Value> = None;
            let mut bin_slice: Option<&[u8]> = None;
            while offset + 8 <= bytes.len() {
                let len = u32::from_le_bytes([
                    bytes[offset],
                    bytes[offset + 1],
                    bytes[offset + 2],
                    bytes[offset + 3],
                ]) as usize;
                let kind = u32::from_le_bytes([
                    bytes[offset + 4],
                    bytes[offset + 5],
                    bytes[offset + 6],
                    bytes[offset + 7],
                ]);
                offset += 8;
                let end = offset.checked_add(len).ok_or("GLB chunk length overflow")?;
                if end > bytes.len() {
                    return Err("Truncated GLB chunk".into());
                }
                let data = &bytes[offset..end];
                if kind == 0x4E4F_534A && json_val.is_none() {
                    json_val = Some(
                        serde_json::from_slice(data)
                            .map_err(|e| format!("Invalid GLB JSON chunk: {e}"))?,
                    );
                } else if kind == 0x004E_4942 && bin_slice.is_none() {
                    bin_slice = Some(data);
                }
                offset = end;
            }
            (json_val.ok_or("GLB missing JSON chunk")?, bin_slice)
        } else {
            let v: serde_json::Value =
                serde_json::from_slice(bytes).map_err(|e| format!("Invalid glTF JSON: {e}"))?;
            (v, None)
        };

    // Resolve buffer bytes.
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    if let Some(arr) = json.get("buffers").and_then(|v| v.as_array()) {
        for (i, buf) in arr.iter().enumerate() {
            if let Some(uri) = buf.get("uri").and_then(|u| u.as_str()) {
                if let Some(rest) = uri.strip_prefix("data:") {
                    let b64 = rest
                        .split_once(',')
                        .map(|(_, d)| d)
                        .ok_or("Invalid data URI in glTF buffer")?;
                    buffers.push(decode_base64(b64)?);
                } else {
                    let rel = uri.replace("%20", " ");
                    let data = std::fs::read(base_dir.join(&rel))
                        .map_err(|e| format!("Could not read glTF buffer {rel}: {e}"))?;
                    buffers.push(data);
                }
            } else if i == 0 {
                buffers.push(bin_chunk.map(|b| b.to_vec()).unwrap_or_default());
            } else {
                buffers.push(Vec::new());
            }
        }
    } else if let Some(b) = bin_chunk {
        buffers.push(b.to_vec());
    }

    let nodes = json
        .get("nodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let meshes = json
        .get("meshes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let materials = json
        .get("materials")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Determine root node indices from the active scene (or nodes without parents).
    let mut roots: Vec<usize> = Vec::new();
    let scene_idx = json.get("scene").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    if let Some(scene_nodes) = json
        .get("scenes")
        .and_then(|v| v.as_array())
        .and_then(|s| s.get(scene_idx))
        .and_then(|s| s.get("nodes"))
        .and_then(|n| n.as_array())
    {
        for n in scene_nodes {
            if let Some(idx) = n.as_u64() {
                roots.push(idx as usize);
            }
        }
    }
    if roots.is_empty() && !nodes.is_empty() {
        let mut children_set: HashSet<usize> = HashSet::new();
        for node in &nodes {
            if let Some(kids) = node.get("children").and_then(|c| c.as_array()) {
                for k in kids {
                    if let Some(idx) = k.as_u64() {
                        children_set.insert(idx as usize);
                    }
                }
            }
        }
        for i in 0..nodes.len() {
            if !children_set.contains(&i) {
                roots.push(i);
            }
        }
    }

    // Compute world transforms for all reachable nodes.
    let mut world_xforms: HashMap<usize, Mat4> = HashMap::new();
    let mut stack: Vec<(usize, Mat4)> = roots.into_iter().rev().map(|r| (r, Mat4::IDENTITY)).collect();
    while let Some((idx, parent_xf)) = stack.pop() {
        let Some(node) = nodes.get(idx) else { continue };
        let local = node_local_mat4(node);
        let world = parent_xf * local;
        world_xforms.insert(idx, world);
        if let Some(kids) = node.get("children").and_then(|c| c.as_array()) {
            for k in kids.iter().rev() {
                if let Some(ki) = k.as_u64() {
                    stack.push((ki as usize, world));
                }
            }
        }
    }

    let mut out = Vec::new();
    if nodes.is_empty() {
        // Fallback: load meshes directly if there are no nodes.
        for (mi, mesh) in meshes.iter().enumerate() {
            let name = mesh
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Mesh")
                .to_string();
            extract_gltf_mesh(
                &json,
                &buffers,
                mesh,
                &materials,
                &name,
                mi,
                Mat4::IDENTITY,
                &mut out,
            )?;
        }
    } else {
        for (ni, node) in nodes.iter().enumerate() {
            let Some(mesh_idx) = node.get("mesh").and_then(|v| v.as_u64()).map(|v| v as usize) else {
                continue;
            };
            let Some(mesh) = meshes.get(mesh_idx) else { continue };
            let world = world_xforms.get(&ni).copied().unwrap_or(Mat4::IDENTITY);
            let name = node
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| mesh.get("name").and_then(|v| v.as_str()))
                .unwrap_or("Mesh")
                .to_string();
            extract_gltf_mesh(&json, &buffers, mesh, &materials, &name, ni, world, &mut out)?;
        }
    }

    if out.is_empty() {
        return Err("glTF file contains no triangle meshes".into());
    }
    Ok(out)
}

fn node_local_mat4(node: &serde_json::Value) -> Mat4 {
    if let Some(m) = node.get("matrix").and_then(|v| v.as_array()) {
        if m.len() == 16 {
            let mut cols = [0.0f32; 16];
            for (i, v) in m.iter().enumerate() {
                cols[i] = v.as_f64().unwrap_or(0.0) as f32;
            }
            return Mat4::from_cols_array(&cols);
        }
    }
    let t = node
        .get("translation")
        .and_then(|v| v.as_array())
        .filter(|a| a.len() == 3)
        .map(|a| {
            Vec3::new(
                a[0].as_f64().unwrap_or(0.0) as f32,
                a[1].as_f64().unwrap_or(0.0) as f32,
                a[2].as_f64().unwrap_or(0.0) as f32,
            )
        })
        .unwrap_or(Vec3::ZERO);
    let r = node
        .get("rotation")
        .and_then(|v| v.as_array())
        .filter(|a| a.len() == 4)
        .map(|a| {
            Quat::from_xyzw(
                a[0].as_f64().unwrap_or(0.0) as f32,
                a[1].as_f64().unwrap_or(0.0) as f32,
                a[2].as_f64().unwrap_or(0.0) as f32,
                a[3].as_f64().unwrap_or(1.0) as f32,
            )
            .normalize()
        })
        .unwrap_or(Quat::IDENTITY);
    let s = node
        .get("scale")
        .and_then(|v| v.as_array())
        .filter(|a| a.len() == 3)
        .map(|a| {
            Vec3::new(
                a[0].as_f64().unwrap_or(1.0) as f32,
                a[1].as_f64().unwrap_or(1.0) as f32,
                a[2].as_f64().unwrap_or(1.0) as f32,
            )
        })
        .unwrap_or(Vec3::ONE);
    Mat4::from_scale_rotation_translation(s, r, t)
}

#[allow(clippy::too_many_arguments)]
fn extract_gltf_mesh(
    json: &serde_json::Value,
    buffers: &[Vec<u8>],
    mesh: &serde_json::Value,
    materials: &[serde_json::Value],
    base_name: &str,
    _idx: usize,
    world: Mat4,
    out: &mut Vec<ImportedMesh>,
) -> Result<(), String> {
    let Some(prims) = mesh.get("primitives").and_then(|v| v.as_array()) else {
        return Ok(());
    };
    let normal_mat = Mat3::from_mat4(world).inverse().transpose();
    for (pi, prim) in prims.iter().enumerate() {
        let mode = prim.get("mode").and_then(|v| v.as_u64()).unwrap_or(4);
        if mode != 4 {
            continue;
        }
        let Some(attrs) = prim.get("attributes") else { continue };
        let Some(pos_acc) = attrs.get("POSITION").and_then(|v| v.as_u64()).map(|v| v as usize) else {
            continue;
        };
        let raw_pos = read_vec3_accessor(json, buffers, pos_acc)?;
        if raw_pos.len() < 3 {
            continue;
        }
        let raw_norm = attrs
            .get("NORMAL")
            .and_then(|v| v.as_u64())
            .and_then(|idx| read_vec3_accessor(json, buffers, idx as usize).ok());
        let raw_uv = attrs
            .get("TEXCOORD_0")
            .and_then(|v| v.as_u64())
            .and_then(|idx| read_vec2_accessor(json, buffers, idx as usize).ok());
        let indices: Vec<u32> = match prim.get("indices").and_then(|v| v.as_u64()) {
            Some(idx) => read_indices_accessor(json, buffers, idx as usize)?,
            None => (0..raw_pos.len() as u32).collect(),
        };

        let mut positions = Vec::with_capacity(indices.len());
        let mut normals = Vec::with_capacity(indices.len());
        let mut uvs = Vec::with_capacity(indices.len());
        for tri in indices.chunks_exact(3) {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            if i0 >= raw_pos.len() || i1 >= raw_pos.len() || i2 >= raw_pos.len() {
                continue;
            }
            let p0 = world.transform_point3(raw_pos[i0]);
            let p1 = world.transform_point3(raw_pos[i1]);
            let p2 = world.transform_point3(raw_pos[i2]);
            let flat_n = (p1 - p0).cross(p2 - p0).normalize_or_zero();
            positions.extend_from_slice(&[p0, p1, p2]);
            for &vi in &[i0, i1, i2] {
                let n = raw_norm
                    .as_ref()
                    .and_then(|ns| ns.get(vi).copied())
                    .map(|n| (normal_mat * n).normalize_or_zero())
                    .filter(|n| n.length_squared() > 1e-6)
                    .unwrap_or(flat_n);
                normals.push(n);
                uvs.push(
                    raw_uv
                        .as_ref()
                        .and_then(|us| us.get(vi).copied())
                        .unwrap_or(Vec2::ZERO),
                );
            }
        }
        if positions.is_empty() {
            continue;
        }
        let color = prim
            .get("material")
            .and_then(|v| v.as_u64())
            .and_then(|mi| materials.get(mi as usize))
            .and_then(|m| m.get("pbrMetallicRoughness"))
            .and_then(|p| p.get("baseColorFactor"))
            .and_then(|c| c.as_array())
            .filter(|a| a.len() >= 3)
            .map(|a| {
                [
                    linear_to_srgb(a[0].as_f64().unwrap_or(0.64) as f32),
                    linear_to_srgb(a[1].as_f64().unwrap_or(0.64) as f32),
                    linear_to_srgb(a[2].as_f64().unwrap_or(0.64) as f32),
                ]
            });
        let name = if prims.len() > 1 {
            format!("{base_name}_{pi}")
        } else {
            base_name.to_string()
        };
        out.push(ImportedMesh {
            name,
            positions,
            normals,
            uvs,
            color,
        });
    }
    Ok(())
}

fn accessor_slice<'a>(
    json: &serde_json::Value,
    buffers: &'a [Vec<u8>],
    acc_idx: usize,
    elem_bytes: usize,
) -> Result<(&'a [u8], usize, usize, u64), String> {
    let acc = json
        .get("accessors")
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(acc_idx))
        .ok_or_else(|| format!("glTF missing accessor {acc_idx}"))?;
    let bv_idx = acc
        .get("bufferView")
        .and_then(|v| v.as_u64())
        .ok_or("glTF accessor missing bufferView")? as usize;
    let bv = json
        .get("bufferViews")
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(bv_idx))
        .ok_or_else(|| format!("glTF missing bufferView {bv_idx}"))?;
    let buf_idx = bv.get("buffer").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let buf = buffers
        .get(buf_idx)
        .ok_or_else(|| format!("glTF missing buffer {buf_idx}"))?;
    let bv_offset = bv.get("byteOffset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let acc_offset = acc.get("byteOffset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let count = acc.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let stride = bv
        .get("byteStride")
        .and_then(|v| v.as_u64())
        .map(|s| s as usize)
        .unwrap_or(elem_bytes)
        .max(elem_bytes);
    let comp = acc.get("componentType").and_then(|v| v.as_u64()).unwrap_or(0);
    let start = bv_offset.checked_add(acc_offset).ok_or("glTF offset overflow")?;
    if count > 0 {
        let last_end = start
            .checked_add((count - 1).saturating_mul(stride))
            .and_then(|v| v.checked_add(elem_bytes))
            .ok_or("glTF buffer bounds overflow")?;
        if last_end > buf.len() {
            return Err("glTF accessor out of buffer bounds".into());
        }
    }
    Ok((&buf[start..], count, stride, comp))
}

fn read_vec3_accessor(
    json: &serde_json::Value,
    buffers: &[Vec<u8>],
    acc_idx: usize,
) -> Result<Vec<Vec3>, String> {
    let (slice, count, stride, comp) = accessor_slice(json, buffers, acc_idx, 12)?;
    if comp != 5126 {
        return Err(format!("Unsupported glTF VEC3 componentType {comp}"));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = i * stride;
        let x = f32::from_le_bytes([slice[o], slice[o + 1], slice[o + 2], slice[o + 3]]);
        let y = f32::from_le_bytes([slice[o + 4], slice[o + 5], slice[o + 6], slice[o + 7]]);
        let z = f32::from_le_bytes([slice[o + 8], slice[o + 9], slice[o + 10], slice[o + 11]]);
        out.push(Vec3::new(x, y, z));
    }
    Ok(out)
}

fn read_vec2_accessor(
    json: &serde_json::Value,
    buffers: &[Vec<u8>],
    acc_idx: usize,
) -> Result<Vec<Vec2>, String> {
    let (slice, count, stride, comp) = accessor_slice(json, buffers, acc_idx, 8)?;
    if comp != 5126 {
        return Err(format!("Unsupported glTF VEC2 componentType {comp}"));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = i * stride;
        let u = f32::from_le_bytes([slice[o], slice[o + 1], slice[o + 2], slice[o + 3]]);
        let v = f32::from_le_bytes([slice[o + 4], slice[o + 5], slice[o + 6], slice[o + 7]]);
        out.push(Vec2::new(u, v));
    }
    Ok(out)
}

fn read_indices_accessor(
    json: &serde_json::Value,
    buffers: &[Vec<u8>],
    acc_idx: usize,
) -> Result<Vec<u32>, String> {
    let acc = json
        .get("accessors")
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(acc_idx))
        .ok_or_else(|| format!("glTF missing index accessor {acc_idx}"))?;
    let comp = acc.get("componentType").and_then(|v| v.as_u64()).unwrap_or(0);
    let elem_bytes = match comp {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        _ => return Err(format!("Unsupported glTF index componentType {comp}")),
    };
    let (slice, count, stride, _) = accessor_slice(json, buffers, acc_idx, elem_bytes)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = i * stride;
        let idx = match comp {
            5121 => slice[o] as u32,
            5123 => u16::from_le_bytes([slice[o], slice[o + 1]]) as u32,
            5125 => u32::from_le_bytes([slice[o], slice[o + 1], slice[o + 2], slice[o + 3]]),
            _ => unreachable!(),
        };
        out.push(idx);
    }
    Ok(out)
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for b in input.bytes() {
        let val = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' | b'\t' => continue,
            _ => return Err("Invalid base64 character in glTF buffer".into()),
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Ok(out)
}
