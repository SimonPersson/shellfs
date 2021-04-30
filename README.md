# Shellfs

Shellfs is a small utility for creating simple filesystems which transforms
some input data.

## Examples

Mirror a (flac) music collection, exposing metadata as plain text:
```
shellfs --mountpoint /tmp/fuse/ --list 'find ~/kind_of_blue -iname "*.flac"' --transform 'metaflac --export-tags-to=- "$INPUT"'
```

Extract files from a sqlite database with a table `files(name, data)`:
```
shellfs \
	--mountpoint /tmp/fuse \
	--list 'sqlite3 files.db "select distinct name from files"' \
	--transform 'sqlite3 files.db "select quote(data) from files where name = \"wall.png\"" | tr -d "X" | xxd -r -p'
```
