//! `POST /api/chat`: the brief as a short conversation (docs/contracts.md#chatrequest-and-chatreply).
//!
//! Stateless: the client sends the whole conversation every turn. Each turn the agent reads everything the
//! user has said and either asks ONE short question (with quick-reply choices, the first being its default)
//! or, once it knows enough, settles the `Brief` and runs the agent's search -> call sheet steps.
//! - At most `MAX_ASKS` questions, and never about something the text already answers.
//! - Claude if `ANTHROPIC_API_KEY` is set; else (or if Claude fails) the offline mock asks a fixed set of
//!   questions in order: where, how the sun sits in the frame, roof or street.
//! - `go: true` (or the user saying "just search") skips the remaining questions and uses the defaults.
//!
//! The settled brief carries `goals` (docs/contracts.md#goals-and-cost-functions) alongside the old fields:
//! for now one library goal named after the framing (`sun_clear` / `sun_between` / `sun_behind`), which is
//! what an empty `goals` means anyway, so the search is unchanged until core reads goals (task 051).

use super::*;
use contracts::{ChatAsk, ChatMessage, ChatReply, ChatRequest, ChatRole, Goal};

/// The most questions the agent asks before it searches with its defaults.
pub const MAX_ASKS: usize = 3;
/// Request limits, so a client can't run up a huge prompt.
const MAX_MESSAGES: usize = 24;
const MAX_TEXT: usize = 1000;
/// Places whose height maps and photos are already cached (the demo areas): offered first, so a choice is quick.
const READY_PLACES: [&str; 2] = ["Downtown LA", "Princeton"];

pub async fn post_chat(State(st): State<Arc<AppState>>, body: Result<Json<ChatRequest>, JsonRejection>) -> Result<Response, ApiErr> {
    let Json(req) = body.map_err(|e| ApiErr::bad(e.body_text()))?;
    check(&req).map_err(ApiErr::bad)?;
    Ok(Json(chat_with(&st, req, Utc::now(), Claude::from_env(&st.http)).await?).into_response())
}

/// Pure: why a request can't be answered, if it can't.
fn check(req: &ChatRequest) -> Result<(), String> {
    let m = &req.messages;
    match (m.first(), m.last()) {
        (None, _) => return Err("messages is empty: start with what you want to shoot".into()),
        (Some(f), _) if f.role != ChatRole::User => return Err("the first message must be the user's".into()),
        (_, Some(l)) if l.role != ChatRole::User && !req.go => return Err("the last message must be the user's (or set go)".into()),
        _ => {}
    }
    if m.len() > MAX_MESSAGES {
        return Err(format!("too many messages ({} > {MAX_MESSAGES})", m.len()));
    }
    if m.iter().any(|x| x.text.chars().count() > MAX_TEXT) {
        return Err(format!("a message is longer than {MAX_TEXT} characters"));
    }
    if user_text(m).is_empty() {
        return Err("the user hasn't said anything yet: describe the shot".into());
    }
    Ok(())
}

/// Things the agent may know or ask about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Topic {
    Place,
    Date,
    Event,
    Standing,
    Framing,
}

/// What one turn of reading decided: the fields, which of them the user actually said, and the next question.
struct Turn {
    fields: Fields,
    known: Vec<Topic>,
    ask: Option<ChatAsk>,
}

async fn chat_with(st: &AppState, req: ChatRequest, now: DateTime<Utc>, llm: Option<Claude>) -> Result<ChatReply, ApiErr> {
    let text = user_text(&req.messages);
    let asked = asks(&req.messages);
    let today = now.with_timezone(&tz::parse(&req.tz).map_err(ApiErr::bad)?).date_naive();
    let mut log = Log::default();
    let mut model = "mock".to_string();
    let turn = match &llm {
        Some(g) => match claude_turn(g, &req, &text, today, asked).await {
            Ok(t) => {
                model = g.model.clone();
                t
            }
            Err(e) => {
                log.add("chat", format!("Claude couldn't follow the chat ({e}); using the offline reader"));
                mock_turn(&req, &text, today)
            }
        },
        None => mock_turn(&req, &text, today),
    };
    let go = req.go || asked >= MAX_ASKS || wants_go(&req.messages);
    let ask = if go { None } else { turn.ask };

    let (mut brief, place) = settle(st, &text, &turn.fields, req.area.as_ref(), req.date.as_deref(), &req.tz, now, &mut log).await?;
    brief.goals = goals_for(&brief);
    let mut known = turn.known;
    if req.area.is_some() {
        known.push(Topic::Place);
    }
    if req.date.is_some() {
        known.push(Topic::Date);
    }
    let words = understood_marked(&brief, &place, &known, ask.is_some());
    if ask.is_some() {
        return Ok(ChatReply { understood: words, ask, brief: None, sheet: None, model });
    }

    let by = if model == "mock" { "the offline reader".to_string() } else { model.clone() };
    let qs = match asked {
        0 => "no questions".to_string(),
        1 => "1 question".to_string(),
        n => format!("{n} questions"),
    };
    log.add("chat", format!("Understood after {qs}: {} (by {by})", words.join(", ")));
    let sheet = sheet_for(st, brief.clone(), place, llm.as_ref(), log).await?;
    Ok(ChatReply { understood: words, ask: None, brief: Some(brief), sheet: Some(sheet), model })
}

// ---- pure pieces ---------------------------------------------------------------------------------

/// Pure: everything the user has said, in order, as one brief ("fountain at sunset. Downtown LA. Rooftop").
fn user_text(m: &[ChatMessage]) -> String {
    m.iter()
        .filter(|x| x.role == ChatRole::User)
        .map(|x| x.text.trim().trim_end_matches('.'))
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(". ")
}

/// Pure: the goals for a settled brief. One library goal per framing; the seed library (task 051) names them.
fn goals_for(b: &Brief) -> Vec<Goal> {
    let name = match b.framing {
        Framing::Clear => "sun_clear",
        Framing::Between => "sun_between",
        Framing::Behind => "sun_behind",
    };
    vec![Goal::using(name)]
}

/// Pure: how many questions the agent has asked so far.
fn asks(m: &[ChatMessage]) -> usize {
    m.iter().filter(|x| x.role == ChatRole::Agent).count()
}

/// Pure: the user's last message hands the choice to us ("just search", "you pick", "go").
fn wants_go(m: &[ChatMessage]) -> bool {
    const GO: [&str; 9] = ["go", "search", "just search", "search now", "you pick", "you choose", "whatever", "surprise me", "skip"];
    let Some(last) = m.last().filter(|x| x.role == ChatRole::User) else { return false };
    let t: String = last.text.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect();
    GO.contains(&t.split_whitespace().collect::<Vec<_>>().join(" ").as_str())
}

/// The mock's questions, in the order it asks them. Choices: the default first.
fn question(t: Topic) -> ChatAsk {
    let (q, c): (&str, &[&str]) = match t {
        Topic::Place => ("Where should I look?", &["Downtown LA", "Princeton"]),
        Topic::Framing => ("How should the sun sit in the shot?", &["A clear view of the sun", "Sun between two buildings", "Sun behind a building (silhouette)"]),
        Topic::Standing => ("Rooftop or street level?", &["Either is fine", "Rooftop", "Street level"]),
        Topic::Event => ("Sunset or sunrise?", &["Sunset", "Sunrise"]),
        Topic::Date => ("Which day?", &["Tomorrow", "This weekend"]),
    };
    ChatAsk { question: q.into(), choices: c.iter().map(|s| s.to_string()).collect() }
}

const MOCK_ORDER: [Topic; 3] = [Topic::Place, Topic::Framing, Topic::Standing];

fn has_any(text: &str, words: &[&str]) -> bool {
    let t = format!(" {} ", text.to_lowercase());
    words.iter().any(|w| t.contains(w))
}

/// Pure: what the offline reader can tell from the text alone. A topic the mock already asked about counts as
/// known (whatever the answer, its default then stands), so it never asks twice.
fn mock_known(m: &[ChatMessage], text: &str, f: &Fields, today: NaiveDate) -> Vec<Topic> {
    let asked = |t: Topic| m.iter().any(|x| x.role == ChatRole::Agent && x.text == question(t).question);
    let mut k = Vec::new();
    if f.place.is_some() || asked(Topic::Place) {
        k.push(Topic::Place);
    }
    if read::date(text, today).is_some() {
        k.push(Topic::Date);
    }
    if has_any(text, &["sunset", "sunrise", "dawn", "dusk", "morning", "evening", "rising sun", "setting"]) {
        k.push(Topic::Event);
    }
    if f.standing != Standing::Any || has_any(text, &["either", " any", "don't mind", "dont mind"]) || asked(Topic::Standing) {
        k.push(Topic::Standing);
    }
    if f.framing != Framing::Clear || has_any(text, &["clear", "open", "horizon", "anything"]) || asked(Topic::Framing) {
        k.push(Topic::Framing);
    }
    k
}

fn mock_turn(req: &ChatRequest, text: &str, today: NaiveDate) -> Turn {
    let fields = Fields::read(text);
    let mut known = mock_known(&req.messages, text, &fields, today);
    if req.area.is_some() {
        known.push(Topic::Place);
    }
    let ask = MOCK_ORDER.iter().find(|t| !known.contains(t)).map(|t| question(*t));
    Turn { fields, known, ask }
}

/// Pure: the brief in plain words. Things the user didn't say are left out while we're still asking, and
/// marked "(my pick)" once we search, so they can see what we assumed.
fn understood_marked(b: &Brief, place: &Place, known: &[Topic], asking: bool) -> Vec<String> {
    let words = understood(b, place); // event, framing, standing, place, day
    [Topic::Event, Topic::Framing, Topic::Standing, Topic::Place, Topic::Date]
        .iter()
        .zip(words)
        .filter_map(|(t, w)| match (known.contains(t), asking) {
            (true, _) => Some(w),
            (false, true) => None,
            (false, false) => Some(format!("{w} (my pick)")),
        })
        .collect()
}

// ---- Claude ------------------------------------------------------------------------------------

/// Pure: the conversation as the model sees it.
fn transcript(m: &[ChatMessage]) -> String {
    m.iter()
        .map(|x| format!("{}: {}", if x.role == ChatRole::User { "Filmmaker" } else { "You" }, x.text.replace('"', "'")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn chat_prompt(req: &ChatRequest, today: NaiveDate, asked: usize) -> String {
    let places: Vec<&str> = read::PRESETS.iter().map(|p| p.name).collect();
    let area = if req.area.is_some() { "The filmmaker already picked the area on the map, so don't ask where.\n" } else { "" };
    format!(
        "You help a filmmaker find where to stand for a sunset or sunrise shot. Our search needs: place, date, \
         event (sunset or sunrise), standing (roof = rooftop/terrace, ground = street/park, any) and framing \
         (clear = open view of the sun, between = sun in a gap between buildings, behind = sun behind a building, silhouette).\n\
         Today is {} ({}). Places with data: {:?}.\n{area}\
         The conversation so far:\n{}\n\n\
         Return your reading: place = the place named (one of the listed places if it matches, else as written, \"\" if none); \
         date = YYYY-MM-DD if a day is implied (\"late June\" = the 24th of the next June), else \"\"; event, standing, framing \
         (your best guess); known = the fields the filmmaker actually said or clearly implied (NOT your guesses).\n\
         Then either ask ONE short, friendly question (under 15 words) about the most important unknown, or set ready = true.\n\
         Rules: never ask about something already answered. You have asked {asked} of at most {MAX_ASKS} questions. \
         Ask only what changes where they should stand: the place first if unknown, then how the sun sits in the frame, then roof or street. \
         Don't ask about the date (tomorrow is the default) or sunset vs sunrise unless it's unclear and matters. \
         Give 2-4 short choices; the first is the default you'd pick (for the place, offer {} first: their data is ready). If the filmmaker leaves it to you or says to search, ready = true. \
         When ready, question = \"\" and choices = [].",
        today.format("%A %Y-%m-%d"),
        req.tz,
        places,
        transcript(&req.messages),
        READY_PLACES.join(" and "),
    )
}

fn chat_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "place": { "type": "string" },
            "date": { "type": "string" },
            "event": { "type": "string", "enum": ["sunset", "sunrise"] },
            "standing": { "type": "string", "enum": ["roof", "ground", "any"] },
            "framing": { "type": "string", "enum": ["clear", "between", "behind"] },
            "known": { "type": "array", "items": { "type": "string", "enum": ["place", "date", "event", "standing", "framing"] } },
            "ready": { "type": "boolean" },
            "question": { "type": "string" },
            "choices": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["place", "date", "event", "standing", "framing", "known", "ready", "question", "choices"],
        "additionalProperties": false
    })
}

/// Pure: the model's JSON -> a turn. A question is only kept if it's not ready and the question isn't empty.
fn turn_from_model(text: &str, v: &serde_json::Value) -> Turn {
    let fields = Fields::read(text).with_model(v);
    let known: Vec<Topic> = v["known"].as_array().map(|a| a.iter().filter_map(|t| parse_enum(t)).collect()).unwrap_or_default();
    let q = v["question"].as_str().unwrap_or_default().trim();
    let ask = (!v["ready"].as_bool().unwrap_or(true) && !q.is_empty()).then(|| ChatAsk {
        question: q.to_string(),
        choices: v["choices"].as_array().map(|a| a.iter().filter_map(|c| c.as_str()).map(str::trim).filter(|c| !c.is_empty()).take(4).map(str::to_string).collect()).unwrap_or_default(),
    });
    Turn { fields, known, ask }
}

async fn claude_turn(g: &Claude, req: &ChatRequest, text: &str, today: NaiveDate, asked: usize) -> Result<Turn, String> {
    let v = g.json(&chat_prompt(req, today, asked), chat_schema()).await?;
    Ok(turn_from_model(text, &v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs(lines: &[(&str, &str)]) -> Vec<ChatMessage> {
        lines.iter().map(|(r, t)| ChatMessage { role: if *r == "u" { ChatRole::User } else { ChatRole::Agent }, text: t.to_string() }).collect()
    }

    fn req(lines: &[(&str, &str)]) -> ChatRequest {
        ChatRequest { messages: msgs(lines), area: None, date: None, tz: "America/Los_Angeles".into(), go: false }
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
    }

    fn mock_ask(lines: &[(&str, &str)]) -> Option<String> {
        let r = req(lines);
        mock_turn(&r, &user_text(&r.messages), today()).ask.map(|a| a.question)
    }

    const HERO_LA: &str = "Sunset between two towers from a rooftop in Downtown LA";
    const PRINCETON: &str = "Sun setting between two buildings, from a rooftop in Princeton, NJ, tonight";

    #[test]
    fn a_full_brief_needs_no_questions() {
        assert_eq!(mock_ask(&[("u", HERO_LA)]), None);
        assert_eq!(mock_ask(&[("u", PRINCETON)]), None);
    }

    #[test]
    fn the_mock_asks_only_what_the_text_doesnt_say_in_order() {
        let q = |t: Topic| Some(question(t).question);
        assert_eq!(mock_ask(&[("u", "a sunset shot")]), q(Topic::Place));
        assert_eq!(mock_ask(&[("u", "sunset in princeton")]), q(Topic::Framing));
        assert_eq!(mock_ask(&[("u", "sun behind a tower in princeton")]), q(Topic::Standing));
        // answered with a quick reply: the next topic
        let p = question(Topic::Place).question;
        assert_eq!(mock_ask(&[("u", "a sunset shot"), ("a", &p), ("u", "Princeton")]), q(Topic::Framing));
        // an answer it can't read still counts: it never asks the same thing twice
        assert_eq!(mock_ask(&[("u", "a sunset shot"), ("a", &p), ("u", "Mars")]), q(Topic::Framing));
        let f = question(Topic::Framing).question;
        let s = question(Topic::Standing).question;
        assert_eq!(mock_ask(&[("u", "a sunset shot"), ("a", &p), ("u", "Princeton"), ("a", &f), ("u", "Sun between two buildings"), ("a", &s), ("u", "Rooftop")]), None);
    }

    #[test]
    fn quick_replies_read_back_as_their_fields() {
        let f = |t: &str| Fields::read(t);
        assert_eq!(f("Sun between two buildings").framing, Framing::Between);
        assert_eq!(f("Sun behind a building (silhouette)").framing, Framing::Behind);
        assert_eq!(f("A clear view of the sun").framing, Framing::Clear);
        assert_eq!(f("Rooftop").standing, Standing::Roof);
        assert_eq!(f("Street level").standing, Standing::Ground);
        assert_eq!(f("Either is fine").standing, Standing::Any);
        assert_eq!(f("Princeton").place.as_deref(), Some("Princeton"));
        assert_eq!(f("Downtown LA").place.as_deref(), Some("Downtown LA"));
        // every mock choice is understood as answering its question
        for t in MOCK_ORDER {
            for c in question(t).choices {
                let text = format!("a sunset shot. {c}");
                assert!(mock_known(&[], &text, &Fields::read(&text), today()).contains(&t), "{c:?} doesn't answer {t:?}");
            }
        }
    }

    #[test]
    fn user_text_joins_only_the_users_words() {
        let m = msgs(&[("u", "fountain at sunset."), ("a", "Where?"), ("u", "  Downtown LA "), ("u", "")]);
        assert_eq!(user_text(&m), "fountain at sunset. Downtown LA");
        assert_eq!(asks(&m), 1);
    }

    #[test]
    fn go_words_only_when_they_are_the_whole_answer() {
        assert!(wants_go(&msgs(&[("u", "x"), ("a", "Where?"), ("u", "Just search!")])));
        assert!(wants_go(&msgs(&[("u", "you pick")])));
        assert!(!wants_go(&msgs(&[("u", "I want to go up a tower")])));
        assert!(!wants_go(&msgs(&[("u", "go"), ("a", "Where?")])));
    }

    #[test]
    fn requests_are_checked() {
        assert!(check(&req(&[])).unwrap_err().contains("empty"));
        assert!(check(&req(&[("a", "hi"), ("u", "x")])).unwrap_err().contains("first"));
        assert!(check(&req(&[("u", "x"), ("a", "Where?")])).unwrap_err().contains("last"));
        assert!(check(&req(&[("u", "   ")])).unwrap_err().contains("hasn't said"));
        assert!(check(&req(&[("u", &"x".repeat(MAX_TEXT + 1))])).unwrap_err().contains("longer"));
        assert!(check(&req(&[("u", "sunset")])).is_ok());
        let mut go = req(&[("u", "x"), ("a", "Where?")]);
        go.go = true;
        assert!(check(&go).is_ok());
    }

    #[test]
    fn understood_hides_guesses_while_asking_and_marks_them_after() {
        let b = Brief {
            text: String::new(),
            area: Area { lat: 40.3487, lon: -74.6593, radius_m: 300.0 },
            date: "2026-11-15".into(),
            tz: "America/New_York".into(),
            event: Event::Sunset,
            standing: Standing::Any,
            framing: Framing::Between,
            max_spots: 10,
            goals: Vec::new(),
            lens_deg: contracts::DEFAULT_LENS_DEG,
        };
        let p = Place { name: "Princeton".into(), area: b.area.clone(), tz: b.tz.clone() };
        let k = [Topic::Event, Topic::Framing, Topic::Place];
        assert_eq!(understood_marked(&b, &p, &k, true), ["Sunset", "sun between two buildings", "Princeton"]);
        assert_eq!(understood_marked(&b, &p, &k, false), ["Sunset", "sun between two buildings", "roof or street (my pick)", "Princeton", "Sun 15 November (my pick)"]);
    }

    #[test]
    fn model_json_becomes_a_turn() {
        let v = json!({"place": "Princeton", "date": "", "event": "sunset", "standing": "any", "framing": "between",
                       "known": ["place", "framing", "bogus"], "ready": false, "question": " Rooftop or street? ", "choices": ["Either", " ", "Rooftop", "Street", "Bridge", "Boat"]});
        let t = turn_from_model("sun in a gap in princeton", &v);
        assert_eq!(t.fields.place.as_deref(), Some("Princeton"));
        assert_eq!(t.known, [Topic::Place, Topic::Framing]);
        let a = t.ask.unwrap();
        assert_eq!(a.question, "Rooftop or street?");
        assert_eq!(a.choices, ["Either", "Rooftop", "Street", "Bridge"]);
        // ready, or an empty question: no ask
        assert!(turn_from_model("x", &json!({"ready": true, "question": "Where?"})).ask.is_none());
        assert!(turn_from_model("x", &json!({"ready": false, "question": ""})).ask.is_none());
    }

    #[test]
    fn prompt_carries_the_conversation_and_the_question_budget() {
        let r = req(&[("u", "fountain at \"sunset\""), ("a", "Where should I look?")]);
        let p = chat_prompt(&r, today(), 1);
        assert!(p.contains("Filmmaker: fountain at 'sunset'\nYou: Where should I look?"), "{p}");
        assert!(p.contains("You have asked 1 of at most 3 questions"));
        assert!(p.contains("Saturday 2026-11-14"));
        assert!(!p.contains("picked the area"));
        assert!(p.contains("offer Downtown LA and Princeton first"));
    }

    #[test]
    fn goals_name_the_framing() {
        let mut b: Brief = serde_json::from_value(json!({"area": {"lat": 0, "lon": 0, "radius_m": 300}, "date": "2026-11-15", "framing": "between"})).unwrap();
        assert_eq!(serde_json::to_value(goals_for(&b)).unwrap(), json!([{"use": "sun_between"}]));
        b.framing = Framing::Behind;
        assert_eq!(goals_for(&b)[0].name(), "sun_behind");
        b.framing = Framing::Clear;
        assert_eq!(goals_for(&b)[0].name(), "sun_clear");
    }

    // ---- whole turns: synthetic grid, no network ----

    fn now() -> DateTime<Utc> {
        "2026-06-20T18:00:00Z".parse().unwrap()
    }

    /// A request on the synthetic city (its area is given, so no place lookup and no download).
    fn city_req(lines: &[(&str, &str)]) -> ChatRequest {
        let mut r = req(lines);
        r.area = Some(Area { lat: crate::testkit::LAT, lon: crate::testkit::LON, radius_m: 300.0 });
        r.date = Some("2026-06-21".into());
        r
    }

    #[tokio::test]
    async fn mock_asks_then_searches() {
        let st = crate::testkit::state(crate::testkit::city());
        let r = chat_with(&st, city_req(&[("u", "a sunset shot")]), now(), None).await.unwrap();
        let a = r.ask.expect("asks how the sun should sit");
        assert_eq!(a.question, question(Topic::Framing).question);
        assert_eq!(a.choices[0], "A clear view of the sun", "the default comes first");
        assert!(r.sheet.is_none() && r.brief.is_none());
        assert_eq!(r.model, "mock");
        assert!(r.understood.contains(&"Sunset".to_string()), "{:?}", r.understood);

        let f = question(Topic::Framing).question;
        let s = question(Topic::Standing).question;
        let r = chat_with(&st, city_req(&[("u", "a sunset shot"), ("a", &f), ("u", "A clear view of the sun"), ("a", &s), ("u", "Either is fine")]), now(), None)
            .await
            .unwrap();
        assert!(r.ask.is_none());
        let sheet = r.sheet.expect("searched");
        assert_eq!(sheet.spots[0].role, contracts::SpotRole::Primary);
        let brief = r.brief.unwrap();
        assert_eq!(brief.text, "a sunset shot. A clear view of the sun. Either is fine");
        assert_eq!(brief.goals, [Goal::using("sun_clear")], "goals alongside the old fields");
        assert_eq!(brief.framing, Framing::Clear);
        assert!(sheet.steps.first().is_some_and(|s| s.tool == "chat" && s.summary.contains("after 2 questions")), "{:?}", sheet.steps);
    }

    #[tokio::test]
    async fn go_and_the_question_budget_skip_to_the_search() {
        let st = crate::testkit::state(crate::testkit::city());
        let mut r = city_req(&[("u", "a sunset shot")]);
        r.go = true;
        let reply = chat_with(&st, r, now(), None).await.unwrap();
        assert!(reply.ask.is_none() && reply.sheet.is_some());
        assert!(reply.understood.iter().any(|w| w.ends_with("(my pick)")), "{:?}", reply.understood);

        let three = city_req(&[("u", "hmm"), ("a", "Q1?"), ("u", "?"), ("a", "Q2?"), ("u", "?"), ("a", "Q3?"), ("u", "?")]);
        assert!(chat_with(&st, three, now(), None).await.unwrap().sheet.is_some());
    }

    #[tokio::test]
    async fn claude_asks_its_own_question() {
        let reply = json!({"place": "", "date": "", "event": "sunset", "standing": "any", "framing": "clear", "known": ["event"],
                           "ready": false, "question": "Which tower should the fountain sit in front of?", "choices": ["The tallest", "Any"]});
        let (g, seen) = super::super::tests::fake_claude(super::super::tests::answer(&reply.to_string())).await;
        let st = crate::testkit::state(crate::testkit::city());
        let r = chat_with(&st, city_req(&[("u", "fountain in front of a tower at sunset")]), now(), Some(g)).await.unwrap();
        assert_eq!(r.ask.unwrap().question, "Which tower should the fountain sit in front of?");
        assert_eq!(r.model, claude::DEFAULT_MODEL);
        // only what the user said (+ the map's area and date), not the model's guesses
        assert_eq!(r.understood, ["Sunset", "the chosen area", "Sun 21 June"]);
        let body = &seen.lock().unwrap()[0].1;
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("Filmmaker: fountain in front of a tower at sunset"));
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("already picked the area"));
    }

    #[tokio::test]
    async fn claude_failing_falls_back_to_the_mock() {
        let (g, _) = super::super::tests::fake_claude(super::super::tests::answer("not json")).await;
        let st = crate::testkit::state(crate::testkit::city());
        let r = chat_with(&st, city_req(&[("u", "a sunset shot")]), now(), Some(g)).await.unwrap();
        assert_eq!(r.ask.unwrap().question, question(Topic::Framing).question);
        assert_eq!(r.model, "mock");
    }
}
