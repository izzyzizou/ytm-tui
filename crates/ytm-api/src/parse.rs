//! Helpers shared by every InnerTube response parser: text runs, thumbnails, tree walks and
//! continuation tokens. All of them return `None`/empty on unknown shapes instead of failing.

use serde_json::Value;

/// Concatenated text of a `{"runs": [...]}` or `{"simpleText": ...}` object.
pub(crate) fn runs_text(v: &Value) -> String {
    v.get("runs")
        .and_then(Value::as_array)
        .map(|runs| runs.iter().filter_map(|r| r.get("text")?.as_str()).collect::<String>())
        .or_else(|| v.get("simpleText").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

/// URL of the last (largest) entry of a `thumbnails` array.
pub(crate) fn last_thumbnail(v: Option<&Value>) -> Option<String> {
    v?.as_array()?.last()?.get("url")?.as_str().map(str::to_owned)
}

/// `MUSIC_PAGE_TYPE_*` of a run or button's browse endpoint.
pub(crate) fn page_type(v: &Value) -> Option<&str> {
    v.pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")?
        .as_str()
}

/// Depth-first visit of every object key (does not descend into matched renderers' children twice).
pub(crate) fn visit<'a>(v: &'a Value, f: &mut impl FnMut(&str, &'a Value)) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                f(k, child);
                if k != "musicResponsiveListItemRenderer" {
                    visit(child, f);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|c| visit(c, f)),
        _ => {}
    }
}

/// First value stored under `key`, anywhere in the tree.
pub(crate) fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|c| find_key(c, key))),
        Value::Array(items) => items.iter().find_map(|c| find_key(c, key)),
        _ => None,
    }
}

/// The token for the next page of a list renderer (a shelf, grid, section list or playlist
/// panel), in either format InnerTube uses:
///
/// * a trailing `continuationItemRenderer` in the renderer's `contents`/`items` (playlist tracks);
/// * `continuations[0].next*ContinuationData.continuation` (home, library, radio queue).
///
/// `reloadContinuationData` refreshes a shelf rather than extending it, so it is ignored.
/// Either token is fetched by POSTing `{"continuation": token}` to the original endpoint.
pub fn continuation_token(renderer: &Value) -> Option<String> {
    let items = renderer.get("contents").or_else(|| renderer.get("items")).and_then(Value::as_array);
    if let Some(token) = items.and_then(|i| item_token(i)) {
        return Some(token);
    }
    renderer.get("continuations")?.as_array()?.iter().find_map(|c| {
        c.as_object()?
            .iter()
            .find(|(k, _)| k.ends_with("ContinuationData") && *k != "reloadContinuationData")?
            .1
            .get("continuation")?
            .as_str()
            .map(str::to_owned)
    })
}

/// Token from a trailing `continuationItemRenderer`, if the list ends with one.
fn item_token(items: &[Value]) -> Option<String> {
    let endpoint = items.last()?.pointer("/continuationItemRenderer/continuationEndpoint")?;
    if let Some(t) = endpoint.pointer("/continuationCommand/token").and_then(Value::as_str) {
        return Some(t.to_owned());
    }
    // Wrapped variant: one of several commands is the browse continuation.
    endpoint.pointer("/commandExecutorCommand/commands")?.as_array()?.iter().find_map(|c| {
        let cmd = c.get("continuationCommand")?;
        match cmd.get("request").and_then(Value::as_str) {
            None | Some("CONTINUATION_REQUEST_TYPE_BROWSE") => cmd.get("token")?.as_str().map(str::to_owned),
            Some(_) => None,
        }
    })
}

/// The items of a continuation response and the token for the page after it.
#[derive(Debug, Default)]
pub struct Continued<'a> {
    pub items: Vec<&'a Value>,
    pub next: Option<String>,
}

/// Parse a response to `{"continuation": token}`, in either format:
///
/// * `continuationContents.<kind>.{contents|items}` (e.g. `sectionListContinuation`,
///   `musicShelfContinuation`, `gridContinuation`, `playlistPanelContinuation`);
/// * `onResponseReceivedActions[].appendContinuationItemsAction.continuationItems`.
///
/// `items` never includes the trailing `continuationItemRenderer`; its token is in `next`.
pub fn continued(resp: &Value) -> Option<Continued<'_>> {
    if let Some(page) = resp.get("continuationContents").and_then(Value::as_object).and_then(|m| m.values().next()) {
        let items = page.get("contents").or_else(|| page.get("items")).and_then(Value::as_array);
        return Some(Continued { items: list_items(items.map(Vec::as_slice).unwrap_or_default()), next: continuation_token(page) });
    }
    let items = resp.get("onResponseReceivedActions")?.as_array()?.iter().find_map(|a| {
        a.pointer("/appendContinuationItemsAction/continuationItems")
            .or_else(|| a.pointer("/reloadContinuationItemsCommand/continuationItems"))
    })?;
    let items = items.as_array()?;
    Some(Continued { items: list_items(items), next: item_token(items) })
}

fn list_items(items: &[Value]) -> Vec<&Value> {
    items.iter().filter(|i| i.get("continuationItemRenderer").is_none()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(id: &str) -> Value {
        json!({"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": id}}})
    }

    #[test]
    fn token_from_trailing_continuation_item() {
        // Playlist track shelf (recorded 2026-10).
        let shelf = json!({"contents": [row("a"), row("b"), {"continuationItemRenderer": {
            "trigger": "CONTINUATION_TRIGGER_ON_ITEM_PRESCAN_VISIBLE",
            "continuationEndpoint": {"continuationCommand": {"token": "PL-TOKEN", "request": "CONTINUATION_REQUEST_TYPE_BROWSE"}}
        }}]});
        assert_eq!(continuation_token(&shelf).as_deref(), Some("PL-TOKEN"));
    }

    #[test]
    fn token_from_wrapped_command() {
        let shelf = json!({"contents": [row("a"), {"continuationItemRenderer": {"continuationEndpoint": {"commandExecutorCommand": {"commands": [
            {"playlistVotingRefreshPopupCommand": {}},
            {"continuationCommand": {"token": "WRAPPED", "request": "CONTINUATION_REQUEST_TYPE_BROWSE"}}
        ]}}}}]});
        assert_eq!(continuation_token(&shelf).as_deref(), Some("WRAPPED"));
    }

    #[test]
    fn token_from_continuation_data() {
        let home = json!({"contents": [{}], "continuations": [{"nextContinuationData": {"continuation": "HOME"}}]});
        assert_eq!(continuation_token(&home).as_deref(), Some("HOME"));
        let radio = json!({"contents": [], "continuations": [{"nextRadioContinuationData": {"continuation": "RADIO"}}]});
        assert_eq!(continuation_token(&radio).as_deref(), Some("RADIO"));
        let reload = json!({"contents": [], "continuations": [{"reloadContinuationData": {"continuation": "RELOAD"}}]});
        assert_eq!(continuation_token(&reload), None);
        assert_eq!(continuation_token(&json!({"contents": [row("a")]})), None);
    }

    #[test]
    fn continued_old_format() {
        let resp = json!({"continuationContents": {"playlistPanelContinuation": {
            "contents": [{"playlistPanelVideoRenderer": {}}, {"playlistPanelVideoRenderer": {}}],
            "continuations": [{"nextRadioContinuationData": {"continuation": "MORE"}}]
        }}});
        let page = continued(&resp).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next.as_deref(), Some("MORE"));
    }

    #[test]
    fn continued_new_format() {
        let resp = json!({"onResponseReceivedActions": [{"appendContinuationItemsAction": {"continuationItems": [
            row("c"), row("d"),
            {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "NEXT"}}}}
        ]}}]});
        let page = continued(&resp).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next.as_deref(), Some("NEXT"));

        let last = json!({"onResponseReceivedActions": [{"appendContinuationItemsAction": {"continuationItems": [row("e")]}}]});
        let page = continued(&last).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next, None);
        assert!(continued(&json!({"responseContext": {}})).is_none());
    }

    #[test]
    fn text_helpers() {
        assert_eq!(runs_text(&json!({"runs": [{"text": "a"}, {"text": " • "}, {"text": "b"}]})), "a • b");
        assert_eq!(runs_text(&json!({"simpleText": "x"})), "x");
        assert_eq!(runs_text(&json!({})), "");
        assert_eq!(last_thumbnail(Some(&json!([{"url": "s"}, {"url": "l"}]))).as_deref(), Some("l"));
        let run = json!({"navigationEndpoint": {"browseEndpoint": {"browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}});
        assert_eq!(page_type(&run), Some("MUSIC_PAGE_TYPE_ALBUM"));
    }
}
