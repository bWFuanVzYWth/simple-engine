pub use hash_dag::{
    Child, DenseRegion, HashDag as Dag, RootId as Root, SetCell, SubtreeState, UniformFill,
    ZOrder20x3 as Path,
};
pub use hash_dag::{HitBehavior, Ray, RaySample, RaycastResult};

pub fn new() -> Dag {
    Dag::new().unwrap()
}
pub const fn root(dag: &Dag) -> Root {
    dag.root()
}
pub fn fork(dag: &mut Dag, root: Root) -> Root {
    dag.fork_root(root).unwrap()
}
pub fn release(dag: &mut Dag, root: Root) {
    dag.release_root(root).unwrap();
}
pub fn set_cells(dag: &mut Dag, root: Root, cells: &[SetCell]) {
    dag.set_cells(root, cells).unwrap();
}
pub fn fill_uniform(dag: &mut Dag, root: Root, fills: &[UniformFill]) {
    dag.fill_uniform(root, fills).unwrap();
}
pub fn set_dense_regions(dag: &mut Dag, root: Root, regions: &[DenseRegion<'_>]) {
    dag.set_dense_regions(root, regions).unwrap();
}
pub fn value_at(dag: &Dag, root: Root, path: Path) -> u32 {
    dag.value_at(root, path).unwrap()
}
pub fn values_at(dag: &Dag, root: Root, paths: &[Path], output: &mut [u32]) {
    dag.values_at(root, paths, output).unwrap();
}
pub fn subtree_at(dag: &Dag, root: Root, path: Path, lod: u8) -> SubtreeState {
    dag.subtree_at(root, path, lod).unwrap()
}
pub fn subtrees_at(dag: &Dag, root: Root, paths: &[Path], lod: u8, output: &mut [SubtreeState]) {
    dag.subtrees_at(root, paths, lod, output).unwrap();
}
pub fn subtree_child(dag: &Dag, root: Root, path: Path, lod: u8) -> Child {
    dag.subtree_child(root, path, lod).unwrap()
}
pub fn decode_region(dag: &Dag, root: Root, path: Path, lod: u8, output: &mut [u32]) {
    dag.decode_region(root, path, lod, output).unwrap();
}
pub fn decode_plane(
    dag: &Dag,
    root: Root,
    path: Path,
    lod: u8,
    axis: u8,
    plane: usize,
    output: &mut [u32],
) {
    dag.decode_plane(root, path, lod, axis, plane, output)
        .unwrap();
}
pub fn retained_bytes(dag: &Dag) -> Option<usize> {
    Some(dag.retained_bytes())
}
pub fn reject_cells(dag: &mut Dag, root: Root, cells: &[SetCell]) {
    assert!(dag.set_cells(root, cells).is_err());
}
pub fn reject_fills(dag: &mut Dag, root: Root, fills: &[UniformFill]) {
    assert!(dag.fill_uniform(root, fills).is_err());
}
pub fn reject_dense(dag: &mut Dag, root: Root, regions: &[DenseRegion<'_>]) {
    assert!(dag.set_dense_regions(root, regions).is_err());
}
pub fn raycast(
    dag: &Dag,
    root: Root,
    ray: Ray,
    predicate: impl FnMut(RaySample) -> HitBehavior,
) -> RaycastResult {
    dag.raycast(root, ray, predicate).unwrap()
}
pub fn raycast_batch(
    dag: &Dag,
    root: Root,
    rays: &[Ray],
    output: &mut [RaycastResult],
    predicate: impl FnMut(usize, RaySample) -> HitBehavior,
) {
    dag.raycast_batch(root, rays, output, predicate).unwrap();
}
