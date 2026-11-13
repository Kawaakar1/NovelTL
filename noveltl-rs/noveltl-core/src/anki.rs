//! Добавление слова в Anki через дополнение AnkiConnect.

use crate::i18n::{t, tf};
use crate::translators::{Http, HttpError};
use serde_json::{json, Value};

pub const ANKI_URL: &str = "http://127.0.0.1:8765";
pub const DECK: &str = "NovelTL";

/// Один вызов AnkiConnect: result или текст ошибки для пользователя.
fn call(http: &dyn Http, action: &str, params: Value) -> Result<Value, String> {
    let body = json!({"action": action, "version": 6, "params": params});
    let resp = http.post_json(ANKI_URL, &[], &body).map_err(|e| match e {
        HttpError::Other(s) => tf("Anki: {}", &[&s]),
        _ => t("Anki не отвечает: запустите Anki с дополнением AnkiConnect (код 2055492159).").to_string(),
    })?;
    let v: Value = serde_json::from_str(&resp.body).map_err(|_| tf("Anki: {}", &[&format!("HTTP {}", resp.status)]))?;
    match v.get("error").and_then(Value::as_str) {
        Some(err) if err.to_lowercase().contains("duplicate") => Err(t("Эта карточка уже есть в Anki.").to_string()),
        Some(err) => Err(tf("Anki: {}", &[&err])),
        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Добавить карточку в Anki через AnkiConnect: колода DECK (создаётся, если нет), тег "NovelTL".
/// front — слово (с чтением), back — значения и предложение-пример. Err — текст для пользователя.
pub fn add_note(http: &dyn Http, front: &str, back: &str) -> Result<(), String> {
    // колода может уже существовать; настоящую недоступность Anki покажет следующий шаг
    let _ = call(http, "createDeck", json!({"deck": DECK}));
    let models = strings(&call(http, "modelNames", json!({}))?);
    // «Basic», иначе первая не-cloze модель с двумя и более полями
    let candidates = models.iter().filter(|m| *m == "Basic").chain(models.iter().filter(|m| *m != "Basic" && !m.to_lowercase().contains("cloze")));
    let mut chosen = None;
    for m in candidates {
        let fields = strings(&call(http, "modelFieldNames", json!({"modelName": m}))?);
        if fields.len() >= 2 {
            chosen = Some((m.clone(), fields));
            break;
        }
    }
    let (model, fields) = chosen.ok_or_else(|| t("Anki: нет подходящего типа заметки. Нужен тип с двумя и более полями, не Cloze (например, «Basic»).").to_string())?;
    let note = json!({
        "deckName": DECK,
        "modelName": model,
        "fields": {fields[0].as_str(): front, fields[1].as_str(): back},
        "options": {"allowDuplicate": false},
        "tags": ["NovelTL"],
    });
    call(http, "addNote", json!({"note": note})).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translators::HttpResponse;
    use std::sync::Mutex;

    struct Fake {
        replies: Mutex<Vec<Result<HttpResponse, HttpError>>>,
        sent: Mutex<Vec<Value>>,
    }

    impl Fake {
        fn new(replies: Vec<Result<Value, HttpError>>) -> Self {
            let r = replies.into_iter().map(|r| r.map(|v| HttpResponse { status: 200, body: v.to_string() })).collect();
            Fake { replies: Mutex::new(r), sent: Mutex::new(vec![]) }
        }
        fn actions(&self) -> Vec<String> {
            self.sent.lock().unwrap().iter().map(|b| b["action"].as_str().unwrap().to_string()).collect()
        }
    }

    impl Http for Fake {
        fn post_json(&self, _: &str, _: &[(&str, &str)], body: &Value) -> Result<HttpResponse, HttpError> {
            self.sent.lock().unwrap().push(body.clone());
            self.replies.lock().unwrap().remove(0)
        }
        fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
            Err(HttpError::Other("нет".into()))
        }
    }

    fn ok(v: Value) -> Result<Value, HttpError> {
        Ok(json!({"result": v, "error": null}))
    }

    #[test]
    fn success_flow_uses_field_names_from_model() {
        let h = Fake::new(vec![
            ok(json!(1)),
            ok(json!(["Другая", "Basic"])),
            ok(json!(["Лицевая сторона", "Оборотная сторона"])),
            ok(json!(123)),
        ]);
        add_note(&h, "言葉", "word").unwrap();
        assert_eq!(h.actions(), ["createDeck", "modelNames", "modelFieldNames", "addNote"]);
        let sent = h.sent.lock().unwrap();
        assert_eq!(sent[2]["params"]["modelName"], "Basic");
        let note = &sent[3]["params"]["note"];
        assert_eq!(note["deckName"], "NovelTL");
        assert_eq!(note["fields"]["Лицевая сторона"], "言葉");
        assert_eq!(note["fields"]["Оборотная сторона"], "word");
        assert!(note["tags"].as_array().unwrap().iter().any(|t| t == "NovelTL"));
    }

    #[test]
    fn without_basic_takes_first_model() {
        let h = Fake::new(vec![ok(json!(1)), ok(json!(["Первая", "Вторая"])), ok(json!(["Front", "Back"])), ok(json!(1))]);
        add_note(&h, "a", "b").unwrap();
        assert_eq!(h.sent.lock().unwrap()[2]["params"]["modelName"], "Первая");
    }

    #[test]
    fn cloze_and_one_field_models_are_skipped() {
        let h = Fake::new(vec![ok(json!(1)), ok(json!(["Cloze", "Одно", "Другая"])), ok(json!(["Text"])), ok(json!(["Front", "Back"])), ok(json!(1))]);
        add_note(&h, "a", "b").unwrap();
        let sent = h.sent.lock().unwrap();
        assert_eq!(sent[2]["params"]["modelName"], "Одно");
        assert_eq!(sent[3]["params"]["modelName"], "Другая");
        assert_eq!(sent[4]["params"]["note"]["modelName"], "Другая");
    }

    #[test]
    fn only_cloze_gives_clear_error() {
        let h = Fake::new(vec![ok(json!(1)), ok(json!(["Cloze"]))]);
        let e = add_note(&h, "a", "b").unwrap_err();
        assert_eq!(e, t("Anki: нет подходящего типа заметки. Нужен тип с двумя и более полями, не Cloze (например, «Basic»)."));
        assert!(!e.contains("modelNames"));
    }

    #[test]
    fn duplicate_error_is_reported() {
        let dup = Ok(json!({"result": null, "error": "cannot create note because it is a duplicate"}));
        let h = Fake::new(vec![ok(json!(1)), ok(json!(["Basic"])), ok(json!(["Front", "Back"])), dup]);
        assert_eq!(add_note(&h, "a", "b").unwrap_err(), t("Эта карточка уже есть в Anki."));
    }

    #[test]
    fn connection_error_mentions_ankiconnect() {
        let c = || Err(HttpError::Connect("ConnectError".into()));
        let h = Fake::new(vec![c(), c()]);
        assert!(add_note(&h, "a", "b").unwrap_err().contains("AnkiConnect"));
    }

    #[test]
    fn get_bytes_default_is_error() {
        assert!(Fake::new(vec![]).get_bytes("http://x").is_err());
    }
}
