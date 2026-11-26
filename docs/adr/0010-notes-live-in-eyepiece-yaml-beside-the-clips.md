# Notes live in eyepiece.yaml beside the clips, network volumes included

Marks lasted until their tab closed and nothing was written beside the media. The owner asked for them to persist (2026-11-26): each folder gets one `eyepiece.yaml` holding the notes on its clips (groups of marks, each group a range of frames) and a log of the exports made from them, which eyepiece reads when it opens a clip there. A setting turns saving beside the clips off (on by default); off, the notes stay in eyepiece's own data folder. The owner chose saving on network volumes too, so a team on a shared folder sees each other's notes: this is the one write adr 0004 allows on a network path, and it happens only when there is something to save.

The file must never lose or corrupt notes: it is written whole to a temporary file in the same folder and swapped in, and before each save eyepiece reads it again and changes only the clip it saves. A file it cannot read is never overwritten: eyepiece says which line it cannot read and saves to its own data folder until the file is fixed.

## Consequences

- Opening a clip never creates the file; only a note or an export does.
- A folder sent on carries its notes. The file is visible, not hidden, so people see it before they send.
- Two people saving notes on the same clip at the same moment: the last save wins for that clip. Notes on other clips are not touched.
- A renamed clip leaves its notes under the old name. A re-rendered clip keeps them, labeled as made on an earlier version (by its size and modified time).
- Reading the file on a network volume happens when a clip there is opened, which adr 0004 counts as the user asking.
