use serde_json::json;

fn main() {
    let observation_links = json!({
        "url": "https://news.ycombinator.com/",
        "data": {
            "links": [
                {"text": "1. Launch HN: Demo", "url": "https://example.com/demo"},
                {"text": "42 comments", "url": "https://news.ycombinator.com/item?id=1"},
                {"text": "Ask HN: Example question", "url": "https://news.ycombinator.com/item?id=2"},
                {"text": "discuss", "url": "https://news.ycombinator.com/item?id=2"}
            ],
            "paragraphs": [
                {"text": "42 points by alice | 42 comments"},
                {"text": "17 points by bob | discuss"}
            ]
        }
    });

    let observation_text = json!({
        "url": "https://news.ycombinator.com/",
        "text_sample": "1. Show HN: Example Tool\n120 points by carol | 33 comments\nhttps://example.com/tool\n\n2. Ask HN: Anything\n58 points by dan | discuss\nhttps://news.ycombinator.com/item?id=99"
    });

    println!("links -> {:?}", soulbrowser_cli::parsers::parse_hackernews_feed(&observation_links));
    println!("text -> {:?}", soulbrowser_cli::parsers::parse_hackernews_feed(&observation_text));
}
