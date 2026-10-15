//! Binary STL, the mesh format of the pipeline (`mesh/mesh.stl`).

use std::path::Path;

use anyhow::{ensure, Context, Result};

/// Writes an indexed mesh as binary STL with unit facet normals. Fails if the file exists.
///
/// Coordinates are rounded to `float32` first and the normal is computed from the rounded
/// corners in `float32`, as `tsdf_hull_mesh.write_stl` does.
pub fn write_binary(path: &Path, header: &str, vertices: &[[f64; 3]], faces: &[[u32; 3]]) -> Result<()> {
    ensure!(header.len() <= 80, "an STL header holds 80 bytes");
    ensure!(u32::try_from(faces.len()).is_ok(), "too many triangles for binary STL");
    ensure!(!crate::storage::exists(path), "cannot create {}: it exists", path.display());
    let mut stream = Vec::with_capacity(84 + 50 * faces.len());
    let mut head = [b' '; 80];
    head[..header.len()].copy_from_slice(header.as_bytes());
    stream.extend_from_slice(&head);
    stream.extend_from_slice(&(faces.len() as u32).to_le_bytes());
    let mut record = [0u8; 50];
    for face in faces {
        let corner = |i: usize| vertices[face[i] as usize].map(|c| c as f32);
        let (a, b, c) = (corner(0), corner(1), corner(2));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let mut normal = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt().max(1e-20);
        normal = normal.map(|n| n / length);
        for (slot, value) in normal.iter().chain(&a).chain(&b).chain(&c).enumerate() {
            record[slot * 4..slot * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        stream.extend_from_slice(&record);
    }
    crate::storage::write_new(path, stream).with_context(|| format!("cannot create {}", path.display()))
}

/// Three corners, or one normal.
pub type Triangle = [[f32; 3]; 3];
pub type Normal = [f32; 3];

/// Triangles of a binary STL as corner coordinates, and their stored normals.
pub fn read_binary(path: &Path) -> Result<(Vec<Triangle>, Vec<Normal>)> {
    let bytes = crate::storage::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    ensure!(bytes.len() >= 84, "{} is too short for a binary STL", path.display());
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    ensure!(bytes.len() == 84 + count * 50, "{} does not hold the {count} triangles it announces", path.display());
    let number = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let triple = |at: usize| [number(at), number(at + 4), number(at + 8)];
    let mut triangles = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    for record in 0..count {
        let at = 84 + record * 50;
        normals.push(triple(at));
        triangles.push([triple(at + 12), triple(at + 24), triple(at + 36)]);
    }
    Ok((triangles, normals))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_unit_normals_and_no_overwrite() {
        let path = std::env::temp_dir().join(format!("crispmesh-stl-{}.stl", std::process::id()));
        let _ = crate::storage::remove_file(&path);
        let vertices = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 0.0]];
        let faces = [[0, 1, 2], [0, 2, 1], [0, 3, 3]];
        write_binary(&path, "crispmesh tsdf hull mesh", &vertices, &faces).unwrap();
        assert!(write_binary(&path, "again", &vertices, &faces).is_err(), "an existing file must not be replaced");
        let bytes = crate::storage::read(&path).unwrap();
        assert_eq!(bytes.len(), 84 + 3 * 50);
        assert!(bytes.starts_with(b"crispmesh tsdf hull mesh ") && bytes[79] == b' ');
        let (triangles, normals) = read_binary(&path).unwrap();
        crate::storage::remove_file(&path).unwrap();
        assert_eq!(triangles[0], [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 3.0, 0.0]]);
        assert_eq!(normals, vec![[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [0.0, 0.0, 0.0]]);
    }
}
