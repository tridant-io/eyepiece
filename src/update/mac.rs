//! the update on macos: the check alone, through the system's url session. a mac build is updated with its app
//! bundle, which is not built yet, so download, apply and relaunch say so and change nothing.

use super::{split, Release, AGENT, WAIT};
use block2::RcBlock;
use objc2_foundation::{NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURLResponse, NSURLSession, NSURLSessionConfiguration, NSURL};
use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

const NOT_YET: &str = "updates on the mac come with the app bundle, which is not built yet";

pub fn download(_: &Release, _: &Path, _: &mut dyn FnMut(u64, u64), _: &AtomicBool) -> Result<(), String> {
    Err(NOT_YET.to_string())
}

pub fn apply(_: &Release, _: &Path, _: &str) -> Result<usize, String> {
    Err(NOT_YET.to_string())
}

pub fn relaunch(_: &Path, _: &[OsString]) -> Result<(), String> {
    Err(NOT_YET.to_string())
}

/// nothing is ever set aside on a mac
pub fn tidy(_: &Path) -> Vec<String> {
    Vec::new()
}

/// asks for `url` and gives its body, refusing one longer than `limit`. an ephemeral session keeps no cookie, cache
/// or credential, and the request carries the player's user agent and no language of the user's
pub(super) fn fetch(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    split(url)?;
    let target = NSURL::URLWithString(&NSString::from_str(url)).ok_or_else(|| format!("not a url eyepiece asks: {url}"))?;
    let request = NSMutableURLRequest::requestWithURL(&target);
    request.setValue_forHTTPHeaderField(Some(&NSString::from_str(AGENT)), &NSString::from_str("User-Agent"));
    // the system would send the languages the user reads
    request.setValue_forHTTPHeaderField(Some(&NSString::from_str("*")), &NSString::from_str("Accept-Language"));
    request.setHTTPShouldHandleCookies(false);
    request.setTimeoutInterval(WAIT.as_secs_f64());
    let session = NSURLSession::sessionWithConfiguration(&NSURLSessionConfiguration::ephemeralSessionConfiguration());
    let (send, answer) = mpsc::channel();
    // called once, on the session's own queue
    let handler = RcBlock::new(move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
        let _ = send.send(unsafe { answered(data, response, error, limit) });
    });
    let task = unsafe { session.dataTaskWithRequest_completionHandler(&request, &handler) };
    task.resume();
    // the session lets itself go once its one task ends
    session.finishTasksAndInvalidate();
    // the request's own timeout ends it first; this is for a handler that is never called
    answer.recv_timeout(WAIT * 3).unwrap_or_else(|_| {
        task.cancel();
        Err("github did not answer: it timed out".to_string())
    })
}

/// the end of a request, as the session hands it over
///
/// safety: the three are the completion handler's, each null or alive for the call
unsafe fn answered(data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError, limit: usize) -> Result<Vec<u8>, String> {
    if let Some(error) = error.as_ref() {
        return Err(format!("github did not answer: {}", error.localizedDescription()));
    }
    let response = response.as_ref().ok_or("github did not answer")?;
    // redirects are followed: they must have ended where the player may ask
    let at = response.URL().and_then(|url| url.absoluteString()).map(|url| url.to_string()).unwrap_or_default();
    split(&at)?;
    let status = response.downcast_ref::<NSHTTPURLResponse>().map_or(0, |http| http.statusCode());
    if status != 200 {
        return Err(format!("github answered {status}"));
    }
    let body = data.as_ref().map(|data| data.to_vec()).unwrap_or_default();
    if body.len() > limit {
        return Err("the answer is too long for a release manifest".to_string());
    }
    Ok(body)
}
