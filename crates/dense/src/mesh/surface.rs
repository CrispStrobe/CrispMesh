//! Operations on the indexed triangle mesh: largest component, Taubin smoothing, topology.

use anyhow::{ensure, Result};
use rayon::prelude::*;

pub type Vertex = [f64; 3];
pub type Face = [u32; 3];

/// Result of `largest_component`.
pub struct Component {
    pub vertices: Vec<Vertex>,
    pub faces: Vec<Face>,
    /// Connected components of the whole mesh.
    pub components: usize,
    pub discarded_faces: usize,
}

fn root(parent: &mut [u32], mut node: u32) -> u32 {
    while parent[node as usize] != node {
        parent[node as usize] = parent[parent[node as usize] as usize];
        node = parent[node as usize];
    }
    node
}

/// Keeps the connected component with the most triangles (the one containing the lowest
/// vertex index if several tie) and renumbers its vertices in their original order.
pub fn largest_component(vertices: &[Vertex], faces: &[Face]) -> Result<Component> {
    largest_component_with(vertices, faces).map(|(component, _)| component)
}

/// [`largest_component`] and, for every kept vertex, its index in `vertices`.
pub fn largest_component_with(vertices: &[Vertex], faces: &[Face]) -> Result<(Component, Vec<u32>)> {
    ensure!(!faces.is_empty(), "the field has no zero crossing, so there is no surface to extract");
    let mut parent: Vec<u32> = (0..vertices.len() as u32).collect();
    for face in faces {
        let mut a = root(&mut parent, face[0]);
        for &other in &face[1..] {
            let b = root(&mut parent, other);
            // The smaller index becomes the root, so a root is its component's lowest vertex.
            parent[a.max(b) as usize] = a.min(b);
            a = a.min(b);
        }
    }
    let label: Vec<u32> = (0..vertices.len() as u32).map(|v| root(&mut parent, v)).collect();
    let components = label.iter().enumerate().filter(|(v, &l)| *v as u32 == l).count();
    let mut size = vec![0usize; vertices.len()];
    for face in faces {
        size[label[face[0] as usize] as usize] += 1;
    }
    let mut best = 0;
    for (candidate, &count) in size.iter().enumerate() {
        if count > size[best] {
            best = candidate;
        }
    }
    let mut renumbered = vec![u32::MAX; vertices.len()];
    let mut kept_vertices = Vec::new();
    let mut origin = Vec::new();
    for (v, &l) in label.iter().enumerate() {
        if l as usize == best {
            renumbered[v] = kept_vertices.len() as u32;
            kept_vertices.push(vertices[v]);
            origin.push(v as u32);
        }
    }
    let kept_faces: Vec<Face> =
        faces.iter().filter(|f| label[f[0] as usize] as usize == best).map(|f| f.map(|v| renumbered[v as usize])).collect();
    let discarded_faces = faces.len() - kept_faces.len();
    Ok((Component { vertices: kept_vertices, faces: kept_faces, components, discarded_faces }, origin))
}

/// Distinct neighbours of every vertex in compressed rows: `(start, neighbour)`.
pub(crate) fn adjacency(vertex_count: usize, faces: &[Face]) -> (Vec<usize>, Vec<u32>) {
    let mut start = vec![0usize; vertex_count + 1];
    for face in faces {
        for &v in face {
            start[v as usize + 1] += 2;
        }
    }
    for v in 0..vertex_count {
        start[v + 1] += start[v];
    }
    let mut cursor = start.clone();
    let mut neighbour = vec![0u32; start[vertex_count]];
    for face in faces {
        for k in 0..3 {
            let (a, b) = (face[k], face[(k + 1) % 3]);
            neighbour[cursor[a as usize]] = b;
            cursor[a as usize] += 1;
            neighbour[cursor[b as usize]] = a;
            cursor[b as usize] += 1;
        }
    }
    // Sort each row and drop repeats (an interior edge is listed by both of its triangles).
    let mut compact_start = vec![0usize; vertex_count + 1];
    let mut write = 0;
    for v in 0..vertex_count {
        let (from, to) = (start[v], start[v + 1]);
        neighbour[from..to].sort_unstable();
        let mut last = None;
        for read in from..to {
            let value = neighbour[read];
            if last != Some(value) {
                neighbour[write] = value;
                write += 1;
                last = Some(value);
            }
        }
        compact_start[v + 1] = write;
    }
    neighbour.truncate(write);
    (compact_start, neighbour)
}

/// Taubin smoothing with uniform weights: each cycle moves every vertex towards the mean of
/// its neighbours by `lam`, then by `mu` (negative, so the shape does not shrink).
pub fn taubin(vertices: &mut [Vertex], faces: &[Face], cycles: usize, lam: f64, mu: f64) {
    if cycles == 0 || vertices.is_empty() {
        return;
    }
    let (start, neighbour) = adjacency(vertices.len(), faces);
    let mut other = vec![[0.0; 3]; vertices.len()];
    let step = |from: &[Vertex], to: &mut [Vertex], factor: f64| {
        to.par_iter_mut().enumerate().for_each(|(v, out)| {
            let row = &neighbour[start[v]..start[v + 1]];
            let inverse = 1.0 / (row.len().max(1) as f64);
            let mut mean = [0.0; 3];
            for &n in row {
                for axis in 0..3 {
                    mean[axis] += inverse * from[n as usize][axis];
                }
            }
            for axis in 0..3 {
                out[axis] = from[v][axis] + factor * (mean[axis] - from[v][axis]);
            }
        });
    };
    for _ in 0..cycles {
        step(vertices, &mut other, lam);
        step(&other, vertices, mu);
    }
}

/// The numbers `tsdf_hull_mesh.topology` reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Topology {
    pub vertices: usize,
    pub triangles: usize,
    pub boundary_edges: usize,
    pub nonmanifold_edges: usize,
    pub genus: i64,
    pub signed_volume: f64,
}

pub fn topology(vertices: &[Vertex], faces: &[Face]) -> Topology {
    let mut edges: Vec<u64> = Vec::with_capacity(faces.len() * 3);
    for face in faces {
        for k in 0..3 {
            let (a, b) = (face[k], face[(k + 1) % 3]);
            edges.push(u64::from(a.min(b)) << 32 | u64::from(a.max(b)));
        }
    }
    edges.par_sort_unstable();
    let (mut distinct, mut boundary, mut nonmanifold) = (0usize, 0usize, 0usize);
    let mut at = 0;
    while at < edges.len() {
        let mut end = at + 1;
        while end < edges.len() && edges[end] == edges[at] {
            end += 1;
        }
        distinct += 1;
        boundary += usize::from(end - at == 1);
        nonmanifold += usize::from(end - at > 2);
        at = end;
    }
    let euler = vertices.len() as i64 - distinct as i64 + faces.len() as i64;
    // Python's round() takes halves to the even neighbour.
    let genus = ((2 - euler) as f64 / 2.0).round_ties_even() as i64;
    let six_volumes: f64 = faces
        .par_iter()
        .map(|face| {
            let [a, b, c] = face.map(|v| vertices[v as usize]);
            a[0] * (b[1] * c[2] - b[2] * c[1]) + a[1] * (b[2] * c[0] - b[0] * c[2]) + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum();
    Topology {
        vertices: vertices.len(),
        triangles: faces.len(),
        boundary_edges: boundary,
        nonmanifold_edges: nonmanifold,
        genus,
        signed_volume: six_volumes / 6.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tetrahedron(offset: f64, first: u32) -> (Vec<Vertex>, Vec<Face>) {
        let vertices = vec![[offset, 0.0, 0.0], [offset + 1.0, 0.0, 0.0], [offset, 1.0, 0.0], [offset, 0.0, 1.0]];
        let faces = [[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]].map(|f: [u32; 3]| f.map(|v| v + first)).to_vec();
        (vertices, faces)
    }

    #[test]
    fn topology_of_a_tetrahedron_and_of_an_open_one() {
        let (vertices, mut faces) = tetrahedron(0.0, 0);
        let closed = topology(&vertices, &faces);
        assert_eq!((closed.vertices, closed.triangles, closed.boundary_edges, closed.nonmanifold_edges, closed.genus), (4, 4, 0, 0, 0));
        assert!((closed.signed_volume - 1.0 / 6.0).abs() < 1e-15);
        let flipped: Vec<Face> = faces.iter().map(|f| [f[2], f[1], f[0]]).collect();
        assert!((topology(&vertices, &flipped).signed_volume + 1.0 / 6.0).abs() < 1e-15);
        faces.pop();
        assert_eq!(topology(&vertices, &faces).boundary_edges, 3);
        // A third triangle on one edge makes it non-manifold.
        let (mut vertices, mut faces) = tetrahedron(0.0, 0);
        vertices.push([0.5, 0.5, 2.0]);
        faces.push([0, 1, 4]);
        let fin = topology(&vertices, &faces);
        assert_eq!((fin.nonmanifold_edges, fin.boundary_edges), (1, 2));
    }

    #[test]
    fn largest_component_keeps_the_bigger_part_and_renumbers() {
        // A lone triangle first, then a tetrahedron.
        let mut vertices = vec![[9.0, 9.0, 9.0], [9.0, 8.0, 9.0], [8.0, 9.0, 9.0]];
        let mut faces = vec![[0, 1, 2]];
        let (v, f) = tetrahedron(0.0, 3);
        vertices.extend(v);
        faces.extend(f);
        vertices.push([5.0, 5.0, 5.0]); // an unused vertex counts as a component, as in SciPy
        let part = largest_component(&vertices, &faces).unwrap();
        assert_eq!((part.components, part.discarded_faces), (3, 1));
        assert_eq!(part.vertices.len(), 4);
        assert_eq!(part.vertices[0], [0.0, 0.0, 0.0]);
        assert_eq!(part.faces[0], [0, 2, 1]);
        assert_eq!(topology(&part.vertices, &part.faces).boundary_edges, 0);
        assert!(largest_component(&vertices, &[]).is_err());
    }

    #[test]
    fn taubin_moves_towards_the_neighbour_mean_and_back() {
        let (mut vertices, faces) = tetrahedron(0.0, 0);
        let before = vertices.clone();
        // In a tetrahedron every vertex neighbours the other three.
        taubin(&mut vertices, &faces, 1, 0.5, -0.53);
        let centroid_step = |v: &[Vertex], factor: f64| -> Vec<Vertex> {
            (0..4)
                .map(|i| {
                    let mut out = [0.0; 3];
                    for (axis, slot) in out.iter_mut().enumerate() {
                        let mean = (0..4).filter(|&j| j != i).map(|j| v[j][axis]).sum::<f64>() / 3.0;
                        *slot = v[i][axis] + factor * (mean - v[i][axis]);
                    }
                    out
                })
                .collect()
        };
        let expected = centroid_step(&centroid_step(&before, 0.5), -0.53);
        for (ours, theirs) in vertices.iter().zip(&expected) {
            for axis in 0..3 {
                assert!((ours[axis] - theirs[axis]).abs() < 1e-14);
            }
        }
        let mut unchanged = before.clone();
        taubin(&mut unchanged, &faces, 0, 0.5, -0.53);
        assert_eq!(unchanged, before);
    }
}
