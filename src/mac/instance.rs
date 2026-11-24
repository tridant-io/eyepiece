//! one player per user: a second launch hands its file to the player already open, whose window comes to the
//! front, and exits having made nothing. the players of a user find each other by a socket in the user's own
//! temporary folder: the one that could bind it is the first, and the others write their file's path to it.

use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// the longest path macos takes, in bytes
const LONGEST: usize = 1024;
/// how long a second launch gives the first player to have its window up and take the file
const PATIENCE: Duration = Duration::from_secs(2);

/// whose the run is: this process's, with the socket others will find it by (none when it could not listen, and
/// runs alone), or the player's that was already open and took the file.
pub enum Claim {
    Ours(Option<UnixListener>),
    Theirs,
}

/// where players find each other. `EYEPIECE_INSTANCE` makes a group of its own, as on windows.
fn place() -> PathBuf {
    std::env::temp_dir().join(match std::env::var("EYEPIECE_INSTANCE") {
        Ok(group) => format!("eyepiece-{group}.sock"),
        Err(_) => "eyepiece.sock".to_string(),
    })
}

/// hands `file` to the player already open (with none, it only comes to the front), or makes this process the one
/// others hand theirs to.
pub fn claim(file: Option<&str>) -> Claim {
    claim_at(&place(), file)
}

fn claim_at(place: &Path, file: Option<&str>) -> Claim {
    match UnixListener::bind(place) {
        Ok(listener) => return Claim::Ours(Some(listener)),
        Err(e) if e.kind() != ErrorKind::AddrInUse => return Claim::Ours(None),
        Err(_) => {}
    }
    let Ok(mut first) = UnixStream::connect(place) else {
        // a socket nobody listens on: a player that is gone left it. it is ours to take: one file of our own name
        let _ = std::fs::remove_file(place);
        return Claim::Ours(UnixListener::bind(place).ok());
    };
    // resolved here: the first player runs in another folder. not canonicalized, which would open the file
    let path = match file.map(std::path::absolute) {
        None => PathBuf::new(),
        Some(Ok(path)) => path,
        Some(Err(_)) => return Claim::Ours(None),
    };
    let mut answer = [0];
    let taken = first.set_read_timeout(Some(PATIENCE)).is_ok()
        && first.set_write_timeout(Some(PATIENCE)).is_ok()
        && first.write_all(path.as_os_str().as_bytes()).is_ok()
        && first.shutdown(Shutdown::Write).is_ok()
        && first.read_exact(&mut answer).is_ok()
        && answer == [1];
    // a first player that does not answer: start normally, alone
    if taken { Claim::Theirs } else { Claim::Ours(None) }
}

/// takes what later launches hand over, on a thread of its own: `taken` gets each full path, or an empty one when a
/// launch only wants the window in front. any program of this user can write here, so nothing in it is trusted.
pub fn serve(listener: UnixListener, taken: impl Fn(String) + Send + 'static) {
    let work = move || {
        for mut launch in listener.incoming().flatten() {
            let mut bytes = Vec::new();
            let read = launch.set_read_timeout(Some(PATIENCE)).is_ok() && (&launch).take(LONGEST as u64 + 1).read_to_end(&mut bytes).is_ok();
            let path = String::from_utf8(bytes).ok().filter(|path| read && path.len() <= LONGEST && !path.contains('\0'));
            let _ = launch.write_all(&[path.is_some() as u8]);
            if let Some(path) = path {
                taken(path);
            }
        }
    };
    let _ = std::thread::Builder::new().name("instance".into()).spawn(work);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// a socket of this test's own, removed when the test is done
    struct Place(PathBuf);

    impl Place {
        fn new(name: &str) -> Place {
            Place(std::env::temp_dir().join(format!("eyepiece-test-{}-{name}.sock", std::process::id())))
        }
    }

    impl Drop for Place {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn a_second_launch_hands_its_file_to_the_first_and_makes_nothing() {
        let place = Place::new("first");
        let Claim::Ours(Some(listener)) = claim_at(&place.0, Some("/clips/a.mov")) else { panic!("the first launch is the player") };
        let (tx, rx) = mpsc::channel();
        serve(listener, move |path| {
            let _ = tx.send(path);
        });
        let next = || rx.recv_timeout(Duration::from_secs(5));
        assert!(matches!(claim_at(&place.0, Some("/clips/a b é.mov")), Claim::Theirs));
        assert_eq!(next().as_deref(), Ok("/clips/a b é.mov"));
        // with no file it only asks for the window
        assert!(matches!(claim_at(&place.0, None), Claim::Theirs));
        assert_eq!(next().as_deref(), Ok(""));
        // what is no path is not taken: a nul inside, bytes that are no text, more than any path
        for bytes in [b"/a\0.mov".to_vec(), vec![0xff, 0xfe], vec![b'a'; LONGEST + 1]] {
            let mut other = UnixStream::connect(&place.0).unwrap();
            other.write_all(&bytes).unwrap();
            other.shutdown(Shutdown::Write).unwrap();
            let mut answer = [9];
            other.read_exact(&mut answer).unwrap();
            assert_eq!(answer, [0]);
        }
        assert!(matches!(claim_at(&place.0, Some("/clips/last.mov")), Claim::Theirs));
        assert_eq!(next().as_deref(), Ok("/clips/last.mov"));
    }

    #[test]
    fn a_socket_left_by_a_player_that_is_gone_is_taken_over() {
        let place = Place::new("gone");
        drop(UnixListener::bind(&place.0).unwrap());
        assert!(place.0.exists());
        assert!(matches!(claim_at(&place.0, Some("/clips/a.mov")), Claim::Ours(Some(_))));
    }
}
