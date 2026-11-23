# Network volumes are look-only

On the owner's NAS, opening a file over SMB changes its ctime, and their backup index treats a changed ctime as a changed file and re-hashes it. In November 2026 a read-only scan of the NAS from a workstation did this to more than half a million files. The owner asked for folders to be cached automatically as they are viewed; on a network volume that would be exactly such a scan.

So on a network path the browser uses the folder listing and nothing else. It opens no file until the user opens one, asks for one file's metadata, or asks for previews of the folder, and each of those says what it will read. Playing a network folder asks once, with the file count. Cloud placeholder files and unknown volume types are treated the same way. Local folders cache on their own.

## Consequences

- Hover scrub, posters and the metadata column do not work on a network folder until asked for.
- Code must not call anything that opens a file to learn about it. On Windows `std::fs::metadata` does; use the directory listing's data.
- A test browses a share that logs opens and fails if any are logged. It never runs against the owner's NAS, only a scratch share.
- The owner has not yet said yes to this one in words (it was the recommended default when they said go). If they want network folders cached too, this record is replaced, not edited.
