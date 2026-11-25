# Updates read a plain manifest from the latest release, not the GitHub API

The approved design (`docs/design/settings.html`, section 11) had the update check ask `api.github.com` for the latest release, read the JSON answer with a small reader of our own, then download one zip per system and a `SHA256SUMS.txt`, and unpack the zip with the system's `tar`. Before any of it was built that changed: every release carries one more asset, `eyepiece-release.txt`, the release manifest, written in the settings file's `key = value` form by `scripts\release.ps1`. The player reads it from `https://github.com/tridant-io/eyepiece/releases/latest/download/eyepiece-release.txt`, the form GitHub documents for linking to an asset of the latest release ("Linking to releases", docs.github.com), and downloads each file it names on its own, from `releases/download/v<version>/<name>`.

```text
version = 0.3.0
date = 2026-12-08
notes = added · settings, with a page for each part of the player
file = eyepiece.exe · 6495000 · <the file's sha-256, 64 hex digits>
```

Why:

- No JSON. The manifest is read the way settings.txt is read; a JSON reader would have been the only one in the program.
- No API limit. A download link is not a REST API request, so the 60 unauthenticated requests an hour per address that the design cited do not apply to it.
- Each file is checked on its own: its size and SHA-256 as it lands, and again before it is put in place. A file the install already holds as the release names it is not downloaded at all, so ffmpeg's decoder library (94 MB in `dist\eyepiece` on 2026-11-24) comes down only when it changes.
- Nothing to unpack: no archive, so no `tar`, and no path inside an archive to vet.

## Consequences

- The manifest is the whole of what a release is. A line it cannot read refuses the release; keys it does not know are passed over. File names are plain (letters, digits, `.`, `-`, `_`, ending `.exe`, `.dll` or `.txt`, no device names), and versions are x.y.z, compared number by number: only a strictly greater one is offered.
- The sums come from the same release as the files, so they catch a broken download, not a hostile one. What proves a build is Tridant's is its signature: every `.exe` and `.dll` to be put in place must pass `WinVerifyTrust` and have the running program's signer, compared as the leaf certificate's subject, byte for byte, and the root its chain ends in. Not the thumbprint: Artifact Signing issues a new certificate every few days (the one on `dist\eyepiece\eyepiece.exe` on 2026-11-24 is good from 2026-11-23 to 2026-11-26), so no two builds share one. `scripts\package.ps1` now signs the four ffmpeg libraries with the program for this.
- A build that is not signed itself (a developer build) can check, but never updates itself: it cannot say whose a new build must be.
- `WinVerifyTrust` is asked to fetch nothing: no revocation check, and no certificate the system does not already have. The player sends nothing to any server but GitHub's.
- "Latest" is GitHub's choice: the REST documentation calls it the most recent release that is neither a draft nor a pre-release, by its `created_at`, and a release can be set as latest by hand, or otherwise is labeled by semantic version. Since the player takes only a greater version, a lower one marked latest is never offered.
- Public downloads still wait on ADR 0005; until the repo is public every check gets a 404 and says nothing.
- On macOS only the check exists, until there is an app bundle to update.
