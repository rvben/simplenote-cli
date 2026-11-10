use super::state::Section;
use crate::{
    Error,
    api::{Api, Collection, Http},
};
use serde_json::{Value, json};
use std::sync::mpsc::{self, Receiver, SyncSender};

#[derive(Clone, Debug)]
pub struct Fetch {
    pub generation: u64,
    pub section: Section,
    pub query: String,
    pub cursor: Option<String>,
}
#[derive(Clone)]
pub enum Request {
    Fetch(Fetch),
    Change { id: String, base: u64, patch: Value },
    Pin { id: String, base: u64, pinned: bool },
}
pub enum Reply {
    Page {
        fetch: Fetch,
        result: Result<Value, Error>,
    },
    Written(Result<Value, Error>),
}
pub struct Worker {
    pub requests: SyncSender<Request>,
    pub replies: Receiver<Reply>,
}
impl Worker {
    pub fn start(token: String) -> Self {
        let (requests, incoming) = mpsc::sync_channel::<Request>(1);
        let (outgoing, replies) = mpsc::channel();
        std::thread::spawn(move || {
            let http = Http::new();
            while let Ok(request) = incoming.recv() {
                let fallback = request.clone();
                let reply =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &http {
                        Ok(http) => perform(
                            &Api {
                                http,
                                token: token.clone(),
                            },
                            request,
                        ),
                        Err(_) => match request {
                            Request::Fetch(fetch) => Reply::Page {
                                fetch,
                                result: Err(Error::new(
                                    "internal",
                                    "Could not initialize the connection. Quit and try again.",
                                )),
                            },
                            _ => Reply::Written(Err(Error::new(
                                "internal",
                                "Could not initialize the connection. Nothing was sent.",
                            ))),
                        },
                    }))
                    .unwrap_or_else(|_| match fallback {
                        Request::Fetch(fetch) => Reply::Page {
                            fetch,
                            result: Err(Error::new(
                                "internal",
                                "The connection worker failed. Refresh to try reading again.",
                            )),
                        },
                        Request::Change { id, .. } | Request::Pin { id, .. } => {
                            let mut error = Error::new(
                                "internal",
                                "The connection worker failed while saving.",
                            );
                            error.details = Some(json!({"note_id":id,"outcome_unknown":true}));
                            Reply::Written(Err(error))
                        }
                    });
                if outgoing.send(reply).is_err() {
                    break;
                }
            }
        });
        Self { requests, replies }
    }
}
pub fn perform(api: &Api<'_>, request: Request) -> Reply {
    match request {
        Request::Fetch(fetch) => {
            let tag = if let Section::Tag(tag) = &fetch.section {
                Some(tag.as_str())
            } else {
                None
            };
            let result = api.list_collection(
                50,
                fetch.cursor.as_deref(),
                tag,
                (!fetch.query.is_empty()).then_some(fetch.query.as_str()),
                match fetch.section {
                    Section::Trash => Collection::Trash,
                    Section::Pinned => Collection::Pinned,
                    _ => Collection::Active,
                },
            );
            Reply::Page { fetch, result }
        }
        Request::Change { id, base, patch } => Reply::Written(api.change(&id, base, patch, false)),
        Request::Pin { id, base, pinned } => Reply::Written(api.pin(&id, base, pinned)),
    }
}

pub fn demo_notes() -> Vec<Value> {
    let samples = [
        (
            "A little room to think",
            "# A little room to think\n\nA place for the things worth keeping. Half-formed ideas, small observations, and plans that become something.\n\n## A slower kind of workspace\n\nFind a note on the left. Give it the whole page on the right. When an idea is ready to change, press **e** to open your own editor.\n\n- Keep the useful fragments.\n- Leave room between thoughts.\n- Come back when you need them.\n\n> The best system is the one you want to return to.\n\n## Make yourself at home\n\n`/` searches your notes. `t` opens your tags. `Tab` moves between the index and the page.\n\nThis is a read-only demo. Your account is never contacted.\n",
            vec!["personal", "writing"],
            true,
            false,
            7200,
        ),
        (
            "Small things, lately",
            "# Small things, lately\n\nThe rain before anyone wakes up.\nA second-hand book with someone else's notes.\nThe first good sentence after a long walk.\n\n## To remember\n\n- [x] Take the long way home\n- [ ] Write down the story about the blue door\n- [ ] Send the photographs\n",
            vec!["personal"],
            true,
            false,
            86400,
        ),
        (
            "A weekend by the sea",
            "# A weekend by the sea\n\nNo ambitious itinerary. Just the train, two books, and somewhere to walk.\n\n## Friday\n\nLeave after lunch. Find coffee near the station. Walk until the town gives way to the dunes.\n\n## Bring\n\n- [ ] Walking shoes\n- [ ] The small camera\n- [x] A notebook\n\n## Sunday\n\nBreakfast slowly. One last walk along the water.\n",
            vec!["travel", "personal"],
            false,
            false,
            172800,
        ),
        (
            "Notes on making things",
            "# Notes on making things\n\nMake the smallest thing that tells the truth about the bigger thing.\n\n## Working principles\n\n1. Start with the part someone will touch.\n2. Make the ordinary path feel considered.\n3. Keep the escape hatch obvious.\n\n```rust\nfn keep_going() {\n    let next_step = something_small();\n    next_step.finish();\n}\n```\n\nThe details are where the work becomes yours.\n",
            vec!["work", "writing"],
            false,
            false,
            259200,
        ),
        (
            "The reading shelf",
            "# The reading shelf\n\nA few books to come back to, with room for more.\n\n- The book with the sentence you underlined twice\n- The collection of essays from the little shop\n- The recommendation you wrote on a train ticket\n\n## Questions to carry\n\nWhat changed the way I see something?\nWhat did I disagree with?\nWhat would I like to try?\n",
            vec!["reading"],
            false,
            false,
            345600,
        ),
        (
            "Sunday kitchen",
            "# Sunday kitchen\n\n## Something simple\n\nTomatoes, good bread, olive oil. A handful of herbs from the windowsill.\n\n## Next time\n\n- Less planning\n- More lemon\n- Invite someone over\n",
            vec!["personal"],
            false,
            false,
            432000,
        ),
        (
            "Ideas for the next chapter",
            "# Ideas for the next chapter\n\nAn opening scene in a quiet railway station.\nA letter that arrives a day too late.\nTwo people who remember the same afternoon differently.\n\nNot a plot yet. Enough to start.\n",
            vec!["writing"],
            false,
            false,
            604800,
        ),
        (
            "Old packing list",
            "# Old packing list\n\nThis note is in Trash. In your account, press **u** to restore a trashed note.\n\n- Raincoat\n- Tickets\n- A spare notebook\n",
            vec!["travel"],
            false,
            true,
            1209600,
        ),
    ];
    samples.into_iter().enumerate().map(|(i, (title, content, tags, pinned, deleted, age))| json!({
        "id":format!("demo-{i}"),"version":12-i,"title":title,"content":content,"tags":tags,"pinned":pinned,"markdown":true,"deleted":deleted,"created":1700000000,"modified":1800000000-age
    })).collect()
}
