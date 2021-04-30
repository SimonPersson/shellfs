use clap::Clap;
use fuse::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use libc::ENOENT;
use log::info;
use std::env;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

const TTL: Duration = Duration::from_secs(0);

fn attr(ino: u64, kind: FileType) -> FileAttr {
    FileAttr {
        ino: ino,
        size: 1_000_000_000_000,
        blocks: 1,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind,
        perm: 0o644,
        nlink: 1,
        uid: 0,
        gid: 0,
        rdev: 0,
        flags: 0,
    }
}

#[derive(Debug)]
struct Inode {
    path: PathBuf,
    kind: FileType,
    parent_inode: u64,
}

fn insert_path(inode_map: &mut Vec<Inode>, path: &Path, kind: FileType) -> u64 {
    let parent_inode = if let Some(parent) = path.parent() {
        if let Some((i, _)) = inode_map.iter().enumerate().find(|(_, e)| e.path == parent) {
            (i + 1) as u64
        } else {
            insert_path(inode_map, parent, FileType::Directory)
        }
    } else {
        1
    };

    if path != Path::new("/") && path != Path::new(".") {
        inode_map.push(Inode {
            path: path.to_owned(),
            kind,
            parent_inode,
        });

        inode_map.len() as u64
    } else {
        1
    }
}

#[derive(Clap)]
#[clap(author = clap::crate_authors!(), version = clap::crate_version!())]
struct ShellFS {
    /// Where to mount the file system
    #[clap(short, long)]
    mountpoint: String,
    /// Command which lists the files in the file system
    #[clap(short, long)]
    list: String,
    /// Command which generates the content of each file in the file system
    #[clap(short, long)]
    transform: String,
}

impl ShellFS {
    fn transform(&self, item: &Path) -> Vec<u8> {
        Command::new("sh")
            .arg("-c")
            .arg(&*self.transform)
            .env("INPUT", item.as_os_str())
            .output()
            .expect("Failed to execute transform command.")
            .stdout
    }

    fn items(&self) -> Vec<Inode> {
        let stdout = Command::new("sh")
            .arg("-c")
            .arg(&*self.list)
            .output()
            .expect("Failed to execute list command.")
            .stdout;
        // split stdout into lines
        let stdout = stdout.split(|c| *c == b'\n');
        let os_strs = stdout.map(|s| OsStr::from_bytes(s));
        let os_strs = os_strs.filter(|s| !s.is_empty());
        let mut inode_map = vec![Inode {
            path: PathBuf::from(""),
            kind: FileType::Directory,
            parent_inode: 0,
        }];
        for path in os_strs.map(|s| Path::new(s)) {
            insert_path(&mut inode_map, path, FileType::RegularFile);
        }
        inode_map
    }
}

impl Filesystem for ShellFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        info!("Calling lookup: {} {:?}", parent, name);
        for (
            idx,
            Inode {
                path,
                kind,
                parent_inode,
            },
        ) in self.items().into_iter().enumerate()
        {
            if parent == parent_inode
                && name == path.file_name().expect("child path has no file name")
            {
                reply.entry(&TTL, &attr((idx + 1) as u64, kind), 0);
                return;
            }
        }
        reply.error(ENOENT);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        info!("Calling getattr: {}", ino);
        let items = self.items();
        if ino <= (items.len() as u64) {
            let item = &self.items()[ino as usize - 1];
            reply.attr(&TTL, &attr(ino, item.kind));
        } else {
            reply.error(ENOENT);
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        reply: ReplyData,
    ) {
        info!("Calling read: {} {} {} {}", ino, fh, offset, size);
        let items = self.items();
        if ino > items.len() as u64 {
            reply.error(ENOENT);
        } else {
            let item = &items[ino as usize - 1];
            let data = self.transform(&*item.path);
            let from = (data.len() as i64 - 1).min(offset).max(0) as usize;
            let to = (data.len() as i64).min(offset + size as i64).max(0) as usize;
            reply.data(&data[from..to]);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        info!("Calling readdir: {} {} {}", ino, fh, offset);

        let items = self.items();

        if ino > items.len() as u64 {
            reply.error(ENOENT);
            return;
        }

        let mut entries = vec![
            (ino, FileType::Directory, OsString::from(".")),
            (ino, FileType::Directory, OsString::from("..")),
        ];

        for (idx, inode) in items
            .into_iter()
            .enumerate()
            .filter(|(_, i)| i.parent_inode == ino)
        {
            if let Some(name) = inode.path.file_name() {
                entries.push(((idx + 1) as u64, inode.kind, name.to_owned()));
            }
        }

        for (i, entry) in entries.into_iter().enumerate().skip(offset as usize) {
            // i + 1 means the index of the next entry
            reply.add(entry.0, (i + 1) as i64, entry.1, entry.2);
        }
        reply.ok();
    }
}

fn main() {
    env_logger::init();
    let shellfs = ShellFS::parse();
    let options = ["-o", "ro", "-o", "fsname=hello"]
        .iter()
        .map(|o| o.as_ref())
        .collect::<Vec<&OsStr>>();
    let mountpoint = shellfs.mountpoint.clone();

    daemonize_me::Daemon::new()
        .work_dir(".")
        .start()
        .expect("Couldn't daemonize.");
    fuse::mount(shellfs, mountpoint, &options).unwrap();
}
