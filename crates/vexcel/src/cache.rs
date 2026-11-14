//! On-disk cache of raw Vexcel responses, one GeoTIFF per tile.
//! Root is `VEXCEL_CACHE_DIR`, default `cache/` (gitignored).

use std::path::{Path, PathBuf};

use crate::tiles::TileId;

pub fn default_root() -> PathBuf {
    std::env::var_os("VEXCEL_CACHE_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("cache"))
}

/// `<root>/vexcel/dsm/L<level>/<tx>_<ty>.tif`
pub fn tile_path(root: &Path, t: TileId) -> PathBuf {
    root.join("vexcel").join("dsm").join(format!("L{}", t.level)).join(format!("{}_{}.tif", t.tx, t.ty))
}

pub async fn read(path: &Path) -> Option<Vec<u8>> {
    tokio::fs::read(path).await.ok()
}

/// Write via a temp file and rename, so a crash never leaves half a tile that looks cached.
pub async fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_layout() {
        let p = tile_path(Path::new("c"), TileId { level: 1, tx: 5, ty: -2 });
        assert_eq!(p, Path::new("c").join("vexcel").join("dsm").join("L1").join("5_-2.tif"));
    }

    #[tokio::test]
    async fn write_then_read() {
        let root = std::env::temp_dir().join(format!("vexcel_cache_test_{}", std::process::id()));
        let p = tile_path(&root, TileId { level: 0, tx: 1, ty: 2 });
        assert_eq!(read(&p).await, None);
        write(&p, b"abc").await.unwrap();
        assert_eq!(read(&p).await.as_deref(), Some(&b"abc"[..]));
        std::fs::remove_dir_all(&root).ok();
    }
}
