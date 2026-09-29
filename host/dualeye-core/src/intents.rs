//! Rule-based commands in Italian and English, so the whole voice loop
//! (wake word → transcript → board tools → spoken reply) works before the
//! local LLM of M6, and without one.
//!
//! [`understand`] looks for a few keywords in what Whisper wrote: a face
//! name, a screen, "luminosità"/"brightness", "volume", "ruota"/"rotate",
//! "scrivi"/"write", "temperatura"/"temperature", "che ore"/"what time".
//! It returns the board tools to call and what to answer; anything else is
//! answered with "Non ho capito" / "Sorry, I didn't get that".

use chrono::{Local, Timelike};
use serde_json::{Value, json};

use crate::snapshot::{Face, Snapshot};

/// What the host knows besides the words.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The latest sensor sample, for "how hot is the CPU".
    pub snapshot: Option<Snapshot>,
    /// The speaker's volume, for "louder" (`get_state` has it).
    pub volume: Option<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Board tools to call in order: name and arguments.
    pub calls: Vec<(String, Value)>,
    /// What to say once they have run.
    pub reply: String,
    /// What to say instead if one of them fails.
    pub failure: String,
    /// False when nothing matched: `reply` says so.
    pub understood: bool,
}

/// The lowered, accent-free words of `text`, digits kept together.
fn words(text: &str) -> Vec<String> {
    let plain: String = text
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    plain.split_whitespace().map(str::to_string).collect()
}

fn has(words: &[String], any: &[&str]) -> bool {
    words.iter().any(|w| any.iter().any(|a| a.strip_suffix('*').map_or(w == a, |p| w.starts_with(p))))
}

/// Two words in a row ("che ore", "turn off").
fn has_pair(words: &[String], pairs: &[(&str, &str)]) -> bool {
    words.windows(2).any(|w| pairs.iter().any(|(a, b)| w[0] == *a && w[1].starts_with(b)))
}

const NUMBER_WORDS: &[(&str, u32)] = &[
    ("zero", 0), ("dieci", 10), ("venti", 20), ("trenta", 30), ("quaranta", 40), ("cinquanta", 50), ("sessanta", 60),
    ("settanta", 70), ("ottanta", 80), ("novanta", 90), ("cento", 100), ("ten", 10), ("twenty", 20), ("thirty", 30),
    ("forty", 40), ("fifty", 50), ("sixty", 60), ("seventy", 70), ("eighty", 80), ("ninety", 90), ("hundred", 100),
];

/// The first number, in digits or as a round word.
fn number(words: &[String]) -> Option<u32> {
    words.iter().find_map(|w| w.parse::<u32>().ok().or_else(|| NUMBER_WORDS.iter().find(|(n, _)| w == n).map(|&(_, v)| v)))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    Left,
    Right,
    Both,
}

impl Screen {
    fn of(words: &[String]) -> Screen {
        let left = has(words, &["sinistr*", "left", "cpu", "processore"]);
        let right = has(words, &["destr*", "right", "gpu"]);
        match (left, right) {
            (true, false) if !has(words, &["entramb*", "both", "tutti", "tutte", "all"]) => Screen::Left,
            (false, true) if !has(words, &["entramb*", "both", "tutti", "tutte", "all"]) => Screen::Right,
            _ => Screen::Both,
        }
    }

    fn arg(self) -> &'static str {
        match self {
            Screen::Left => "left",
            Screen::Right => "right",
            Screen::Both => "both",
        }
    }

    fn said(self, it: bool) -> &'static str {
        match (self, it) {
            (Screen::Left, true) => "sullo schermo sinistro",
            (Screen::Right, true) => "sullo schermo destro",
            (Screen::Both, true) => "su entrambi gli schermi",
            (Screen::Left, false) => "on the left screen",
            (Screen::Right, false) => "on the right screen",
            (Screen::Both, false) => "on both screens",
        }
    }
}

/// Face names as Whisper tends to write them.
fn face(words: &[String]) -> Option<Face> {
    let table: &[(&[&str], Face)] = &[
        (&["classic", "classico", "classica", "classici"], Face::Classic),
        // "Rings a sinistra" comes out as "rinza sinistra", "rinusa"...
        (&["ring*", "rink*", "rinz*", "rins*", "rinu*", "anelli"], Face::Rings),
        (&["plus"], Face::Plus),
        (&["bar", "bars", "barra"], Face::Bar),
        (&["clawd", "clawed", "clod", "mascotte", "mascot", "granchio", "crab"], Face::Clawd),
        (&["claude", "cloud", "clode"], Face::Claude),
    ];
    table.iter().find(|(names, _)| has(words, names)).map(|&(_, f)| f)
}

fn plan(calls: Vec<(String, Value)>, it: bool, reply: String) -> Plan {
    let failure = if it { "Non ci sono riuscito." } else { "That didn't work." }.to_string();
    Plan { calls, reply, failure, understood: true }
}

fn tool(name: &str, arguments: Value) -> (String, Value) {
    (name.to_string(), arguments)
}

/// What to do about `text`, said in `language` (`it`, or anything else for English).
pub fn understand(text: &str, language: &str, ctx: &Context) -> Plan {
    let it = language == "it";
    let w = words(text);
    let screen = Screen::of(&w);

    // "Scrivi ciao" / "write hello": the rest of the sentence goes on screen.
    if let Some(rest) = after_verb(text, &["scrivi", "write", "mostra la scritta", "show the text"]) {
        let args = json!({"text": rest, "screen": screen.arg()});
        return plan(vec![tool("show_text", args)], it, if it { "Ecco.".into() } else { "Here it is.".into() });
    }

    if has_pair(&w, &[("che", "or"), ("what", "time")]) || has(&w, &["orario"]) {
        let now = Local::now();
        let (h, m) = (now.hour(), now.minute());
        let reply = match (it, m) {
            (true, 0) => format!("Sono le {h}."),
            (true, _) => format!("Sono le {h} e {m}."),
            (false, _) => format!("It's {h}:{m:02}."),
        };
        return plan(vec![], it, reply);
    }

    if has(&w, &["temperatur*", "caldo", "calda", "hot", "warm", "gradi", "degrees"]) && !has(&w, &["ruota", "gira", "rotate", "turn"]) {
        return plan(vec![], it, temperatures(ctx.snapshot.as_ref(), screen, it));
    }

    let off = has_pair(&w, &[("turn", "off"), ("switch", "off")]) || has(&w, &["spegni"]);
    let on = has_pair(&w, &[("turn", "on"), ("switch", "on")]) || has(&w, &["accendi"]);
    if (off || on) && has(&w, &["schermo", "schermi", "screen", "screens", "display", "luce", "light"]) {
        let pct = if off { 0 } else { 100 };
        let reply = match (it, off) {
            (true, true) => format!("Spento {}.", screen.said(true).replace("sullo", "lo").replace("su entrambi gli", "entrambi gli")),
            (true, false) => "Acceso.".to_string(),
            (false, true) => "Screen off.".to_string(),
            (false, false) => "Screen on.".to_string(),
        };
        return plan(vec![tool("set_brightness", json!({"percent": pct, "screen": screen.arg()}))], it, reply);
    }

    if has(&w, &["luminosit*", "brightness", "bright", "luce"]) {
        let pct = if has(&w, &["massima", "massimo", "max", "maximum", "full", "piena"]) {
            Some(100)
        } else if has(&w, &["minima", "minimo", "min", "minimum"]) {
            Some(10)
        } else if has(&w, &["meta", "half"]) {
            Some(50)
        } else {
            number(&w)
        };
        if let Some(pct) = pct.filter(|p| *p <= 100) {
            let reply = if it { format!("Luminosità al {pct} per cento.") } else { format!("Brightness {pct} percent.") };
            return plan(vec![tool("set_brightness", json!({"percent": pct, "screen": screen.arg()}))], it, reply);
        }
    }

    if has(&w, &["ruota", "gira", "girare", "ruotare", "rotate", "turn", "rotation", "rotazione", "sottosopra"]) || has_pair(&w, &[("upside", "down")]) {
        let degrees = if has(&w, &["sottosopra"]) || has_pair(&w, &[("upside", "down")]) {
            Some(180)
        } else if has(&w, &["dritto", "dritti", "normale", "upright", "normal", "reset"]) {
            Some(0)
        } else {
            number(&w).filter(|d| [0, 90, 180, 270].contains(d))
        };
        // "Turn it up" is the volume.
        if let Some(d) = degrees {
            let reply = if it { format!("Ruotato di {d} gradi.") } else { format!("Turned {d} degrees.") };
            return plan(vec![tool("set_rotation", json!({"degrees": d, "screen": screen.arg()}))], it, reply);
        }
    }

    let louder = has(&w, &["alza", "aumenta", "louder", "up"]) || has_pair(&w, &[("piu", "fort")]);
    let quieter = has(&w, &["abbassa", "diminuisci", "quieter", "down", "lower"]) || has_pair(&w, &[("piu", "pian")]);
    if has(&w, &["volume", "voce", "voice", "parla", "speak"]) || louder || quieter {
        let current = ctx.volume.unwrap_or(60) as i32;
        let pct = if has(&w, &["massimo", "massima", "max", "maximum", "full"]) {
            Some(100)
        } else if let Some(n) = number(&w).filter(|n| *n <= 100) {
            Some(n as i32)
        } else if louder {
            Some((current + 20).min(100))
        } else if quieter {
            Some((current - 20).max(10))
        } else {
            None
        };
        if let Some(pct) = pct {
            let reply = if it { format!("Volume al {pct}.") } else { format!("Volume {pct}.") };
            return plan(vec![tool("set_volume", json!({"percent": pct}))], it, reply);
        }
    }

    if let Some(f) = face(&w) {
        let name = f.name();
        let reply = if it { format!("Fatto: faccia {name} {}.", screen.said(true)) } else { format!("Done: {name} {}.", screen.said(false)) };
        return plan(vec![tool("set_face", json!({"face": name, "screen": screen.arg()}))], it, reply);
    }

    if has_pair(&w, &[("cosa", "sai"), ("what", "can")]) || has(&w, &["aiuto", "help"]) {
        let reply = if it {
            "Posso cambiare faccia, luminosità, rotazione e volume, scrivere un messaggio, dirti le temperature e l'ora."
        } else {
            "I can change the face, brightness, rotation and volume, write a message, and tell you the temperatures and the time."
        };
        return plan(vec![], it, reply.into());
    }

    Plan {
        calls: vec![],
        reply: if it { "Non ho capito." } else { "Sorry, I didn't get that." }.into(),
        failure: String::new(),
        understood: false,
    }
}

/// `it` or `en`, whichever `text` looks more like (for text not heard but
/// typed, which Whisper didn't label).
pub fn guess_language(text: &str) -> &'static str {
    const IT: &[&str] = &["il", "lo", "la", "gli", "le", "di", "che", "e", "un", "una", "per", "non", "sono", "ciao", "questo", "come", "sei", "ho", "del", "della", "con"];
    const EN: &[&str] = &["the", "a", "an", "of", "and", "is", "are", "to", "you", "it", "this", "hello", "what", "i", "in", "on", "with", "for", "not"];
    let w = words(text);
    let count = |list: &[&str]| w.iter().filter(|x| list.contains(&x.as_str())).count();
    if count(IT) > count(EN) || w.iter().any(|x| x.ends_with("zione") || x.ends_with("mente")) && count(EN) == 0 { "it" } else { "en" }
}

/// The original text after one of `verbs`, without the punctuation Whisper
/// adds at the end; ASCII letters only, which the board's font has.
fn after_verb(text: &str, verbs: &[&str]) -> Option<String> {
    let lower = text.to_lowercase();
    let (start, verb) = verbs.iter().filter_map(|v| lower.find(v).map(|i| (i, v))).min()?;
    // A whole word: not "riscrivi", not "writer".
    let before = lower[..start].chars().last();
    if before.is_some_and(char::is_alphanumeric) {
        return None;
    }
    let rest = text.get(start + verb.len()..)?;
    if rest.chars().next().is_some_and(char::is_alphanumeric) {
        return None;
    }
    let rest = rest.trim_start_matches([' ', ':', ',']).trim().trim_end_matches(['.', '!', '?']).trim_matches(['"', '\'', '«', '»']);
    let ascii: String = rest
        .chars()
        .map(|c| match words(&c.to_string()).first().and_then(|w| w.chars().next()) {
            Some(plain) if !c.is_ascii() => plain,
            _ => c,
        })
        .filter(char::is_ascii)
        .collect();
    let ascii = ascii.trim().chars().take(120).collect::<String>();
    (!ascii.is_empty()).then_some(ascii)
}

fn temperatures(snapshot: Option<&Snapshot>, screen: Screen, it: bool) -> String {
    let cpu = snapshot.and_then(|s| s.cpu.temp_c).map(|t| t.round() as i32);
    let gpu = snapshot.and_then(|s| s.gpu.temp_c).map(|t| t.round() as i32);
    let one = |name: &str, t: Option<i32>| match (it, t) {
        (true, Some(t)) => format!("La {name} è a {t} gradi."),
        (true, None) => format!("Non ho la temperatura della {name}."),
        (false, Some(t)) => format!("The {name} is at {t} degrees."),
        (false, None) => format!("I don't have the {name} temperature."),
    };
    match (screen, cpu, gpu) {
        (Screen::Left, ..) => one("CPU", cpu),
        (Screen::Right, ..) => one("GPU", gpu),
        (Screen::Both, Some(c), Some(g)) if it => format!("La CPU è a {c} gradi, la GPU a {g}."),
        (Screen::Both, Some(c), Some(g)) => format!("The CPU is at {c} degrees, the GPU at {g}."),
        (Screen::Both, _, None) => one("CPU", cpu),
        (Screen::Both, None, _) => one("GPU", gpu),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calls(text: &str, lang: &str) -> Vec<(String, Value)> {
        understand(text, lang, &Context::default()).calls
    }

    #[test]
    fn faces_and_screens() {
        assert_eq!(calls("Metti la faccia rings a sinistra.", "it"), [tool("set_face", json!({"face": "rings", "screen": "left"}))]);
        assert_eq!(calls("Put rings on the right screen", "en"), [tool("set_face", json!({"face": "rings", "screen": "right"}))]);
        assert_eq!(calls("Faccia classica su tutti e due gli schermi", "it"), [tool("set_face", json!({"face": "classic", "screen": "both"}))]);
        assert_eq!(calls("Metti la faccia rinza sinistra.", "it"), [tool("set_face", json!({"face": "rings", "screen": "left"}))]);
        assert_eq!(calls("Show Claude on the left", "en"), [tool("set_face", json!({"face": "claude", "screen": "left"}))]);
        let p = understand("Metti la faccia rings a sinistra", "it", &Context::default());
        assert_eq!(p.reply, "Fatto: faccia rings sullo schermo sinistro.");
    }

    #[test]
    fn brightness_volume_rotation() {
        assert_eq!(calls("Luminosità al 40%", "it"), [tool("set_brightness", json!({"percent": 40, "screen": "both"}))]);
        assert_eq!(calls("Set the brightness of the left screen to fifty", "en"), [tool("set_brightness", json!({"percent": 50, "screen": "left"}))]);
        assert_eq!(calls("Spegni lo schermo destro", "it"), [tool("set_brightness", json!({"percent": 0, "screen": "right"}))]);
        assert_eq!(calls("Volume al 30", "it"), [tool("set_volume", json!({"percent": 30}))]);
        let ctx = Context { volume: Some(50), ..Default::default() };
        assert_eq!(understand("Turn it up", "en", &ctx).calls, [tool("set_volume", json!({"percent": 70}))]);
        assert_eq!(understand("Parla più piano", "it", &ctx).calls, [tool("set_volume", json!({"percent": 30}))]);
        assert_eq!(calls("Ruota lo schermo sinistro di 180 gradi", "it"), [tool("set_rotation", json!({"degrees": 180, "screen": "left"}))]);
        assert_eq!(calls("Turn the screens upside down", "en"), [tool("set_rotation", json!({"degrees": 180, "screen": "both"}))]);
    }

    #[test]
    fn text_on_screen() {
        assert_eq!(calls("Scrivi: ciao a tutti!", "it"), [tool("show_text", json!({"text": "ciao a tutti", "screen": "both"}))]);
        assert_eq!(calls("Write hello world.", "en"), [tool("show_text", json!({"text": "hello world", "screen": "both"}))]);
        assert_eq!(calls("Scrivi perché sì", "it")[0].1["text"], "perche si");
    }

    #[test]
    fn guesses_the_language_of_typed_text() {
        assert_eq!(guess_language("Ciao, sono la tua DualEye"), "it");
        assert_eq!(guess_language("Hello, this is your DualEye"), "en");
    }

    #[test]
    fn answers_without_tools() {
        let snap: Snapshot = serde_json::from_value(json!({"v": 2, "ts": 0, "cpu": {"temp_c": 45.4}, "gpu": {"temp_c": 38.0}})).unwrap();
        let ctx = Context { snapshot: Some(snap), ..Default::default() };
        let p = understand("Quanto è calda la CPU?", "it", &ctx);
        assert!(p.calls.is_empty() && p.understood);
        assert_eq!(p.reply, "La CPU è a 45 gradi.");
        assert_eq!(understand("What's the temperature?", "en", &ctx).reply, "The CPU is at 45 degrees, the GPU at 38.");
        assert!(understand("Che ore sono?", "it", &ctx).reply.starts_with("Sono le "));
        let p = understand("Raccontami una barzelletta", "it", &ctx);
        assert!(!p.understood && p.calls.is_empty());
        assert_eq!(p.reply, "Non ho capito.");
    }
}
