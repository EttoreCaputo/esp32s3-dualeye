//! Rule-based commands in Italian and English, so the whole voice loop
//! (wake word → transcript → board tools → spoken reply) works before the
//! local LLM of M6, and without one.
//!
//! [`understand`] looks for a few keywords in what Whisper wrote: a face
//! name, a screen, "luminosità"/"brightness", "volume", "ruota"/"rotate",
//! "scrivi"/"write", "temperatura"/"temperature", "che ore"/"what time",
//! "timer", "ricordami"/"remind me", "pomodoro", "pausa la musica"/"next song",
//! "cosa sta suonando"/"what's playing", "apri Spotify"/"open Safari".
//! It returns the board tools to call and what to answer; anything else is
//! answered with "Non ho capito" / "Sorry, I didn't get that".

use chrono::{Local, NaiveTime, Timelike};
use serde_json::{Value, json};

use crate::music::NowPlaying;
use crate::snapshot::{Face, Snapshot};
use crate::timers::{self, TimerInfo};

/// What the host knows besides the words.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The latest sensor sample, for "how hot is the CPU".
    pub snapshot: Option<Snapshot>,
    /// The speaker's volume, for "louder" (`get_state` has it).
    pub volume: Option<u8>,
    /// The timers running, for "how long is left".
    pub timers: Vec<TimerInfo>,
    /// A timer rings, or rang until the wake word a moment ago: "stop" is for it.
    pub alarm: bool,
    /// What's playing, when the words are about music ([`asks_music`]).
    pub music: Option<NowPlaying>,
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
        (&["net", "network", "rete", "internet"], Face::Net),
        (&["disk", "disco", "dischi"], Face::Disk),
        (&["battery", "batteria"], Face::Battery),
        (&["image", "immagine", "foto", "picture", "photo", "gif"], Face::Image),
        (&["music*", "musica", "canzon*", "song*"], Face::Music),
        (&["occhi", "occhio", "eyes", "eye"], Face::Eyes),
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

    if let Some(plan) = timer_plan(text, &w, it, ctx) {
        return plan;
    }

    if let Some(plan) = app_plan(&w, it) {
        return plan;
    }

    if let Some(plan) = music_plan(&w, it, ctx) {
        return plan;
    }

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
            "Posso cambiare faccia, luminosità, rotazione e volume, scrivere un messaggio, dirti le temperature e l'ora, \
             impostare timer, promemoria e un pomodoro, mettere in pausa o cambiare la musica e aprire le app."
        } else {
            "I can change the face, brightness, rotation and volume, write a message, tell you the temperatures and the time, \
             set timers, reminders and a pomodoro, pause or skip the music and open apps."
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

const UNITS_IT: &[&str] = &["uno", "due", "tre", "quattro", "cinque", "sei", "sette", "otto", "nove"];
const TEENS_IT: &[&str] = &["dieci", "undici", "dodici", "tredici", "quattordici", "quindici", "sedici", "diciassette", "diciotto", "diciannove"];
const TENS_IT: &[(&str, &str, u32)] = &[
    ("venti", "vent", 20), ("trenta", "trent", 30), ("quaranta", "quarant", 40), ("cinquanta", "cinquant", 50),
    ("sessanta", "sessant", 60), ("settanta", "settant", 70), ("ottanta", "ottant", 80), ("novanta", "novant", 90),
];
const UNITS_EN: &[&str] = &["one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
const TEENS_EN: &[&str] = &["ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
const TENS_EN: &[(&str, u32)] = &[("twenty", 20), ("thirty", 30), ("forty", 40), ("fifty", 50), ("sixty", 60), ("seventy", 70), ("eighty", 80), ("ninety", 90)];

/// One number word, Italian or English, up to 99: "venticinque", "ventuno", "fifteen".
fn number_word(w: &str) -> Option<u32> {
    let pos = |list: &[&str]| list.iter().position(|u| *u == w).map(|i| i as u32);
    if let Some(n) = w.parse::<u32>().ok().or_else(|| pos(UNITS_IT).map(|i| i + 1)).or_else(|| pos(UNITS_EN).map(|i| i + 1)) {
        return Some(n);
    }
    if let Some(i) = pos(TEENS_IT).or_else(|| pos(TEENS_EN)) {
        return Some(10 + i);
    }
    if let Some(&(_, n)) = TENS_EN.iter().find(|(t, _)| *t == w) {
        return Some(n);
    }
    if matches!(w, "un" | "una" | "a" | "an") {
        return Some(1);
    }
    TENS_IT.iter().find_map(|&(full, short, n)| {
        let rest = w.strip_prefix(full).or_else(|| w.strip_prefix(short))?;
        if rest.is_empty() {
            return w.starts_with(full).then_some(n);
        }
        UNITS_IT.iter().position(|u| *u == rest).map(|i| n + i as u32 + 1)
    })
}

/// The number at `w[i]`, and how many words it takes ("twenty five": two).
fn number_at(w: &[String], i: usize) -> Option<(u32, usize)> {
    let n = number_word(w.get(i)?)?;
    if TENS_EN.iter().any(|(_, t)| *t == n)
        && let Some(u) = w.get(i + 1).and_then(|u| UNITS_EN.iter().position(|x| x == u))
    {
        return Some((n + u as u32 + 1, 2));
    }
    Some((n, 1))
}

fn unit_secs(w: &str) -> Option<u64> {
    if w.starts_with("second") || w.starts_with("sec") || w == "s" {
        Some(1)
    } else if w.starts_with("minut") || w == "min" || w == "mins" {
        Some(60)
    } else if matches!(w, "ora" | "ore" | "hour" | "hours" | "h" | "orette" | "oretta") {
        Some(3600)
    } else {
        None
    }
}

/// A length said in words: "10 minuti", "un'ora e mezza", "mezz'ora",
/// "un quarto d'ora", "1 ora e 30 minuti", "two and a half minutes"; and the
/// words it took, from..to.
fn duration(w: &[String]) -> Option<(u64, usize, usize)> {
    let mut total = 0u64;
    let (mut first, mut last) = (None, 0usize);
    let mut i = 0;
    while i < w.len() {
        let (secs, used) = if matches!(w[i].as_str(), "mezz" | "mezzora") {
            (1800, if w[i] == "mezz" && w.get(i + 1).is_some_and(|x| x == "ora") { 2 } else { 1 })
        } else if w[i] == "quarto" && w.get(i + 1).is_some_and(|x| x == "d") && w.get(i + 2).is_some_and(|x| x == "ora") {
            (900, 3)
        } else if w[i] == "half" && w.get(i + 1).is_some_and(|x| x == "an") && w.get(i + 2).is_some_and(|x| x.starts_with("hour")) {
            (1800, 3)
        } else if let Some((n, len)) = number_at(w, i) {
            let mut j = i + len;
            // "two and a half minutes"
            let half_before = w.get(j).is_some_and(|x| x == "and") && w.get(j + 1).is_some_and(|x| x == "a") && w.get(j + 2).is_some_and(|x| x == "half");
            if half_before {
                j += 3;
            }
            match w.get(j).and_then(|u| unit_secs(u)) {
                Some(unit) => {
                    let mut secs = n as u64 * unit + if half_before { unit / 2 } else { 0 };
                    // "un minuto e mezzo", "un'ora e mezza", "an hour and a half"
                    let it_half = w.get(j + 1).is_some_and(|x| x == "e") && w.get(j + 2).is_some_and(|x| x.starts_with("mezz"));
                    let en_half = w.get(j + 1).is_some_and(|x| x == "and") && w.get(j + 2).is_some_and(|x| x == "a") && w.get(j + 3).is_some_and(|x| x == "half");
                    let mut used = j + 1 - i;
                    if it_half {
                        secs += unit / 2;
                        used += 2;
                    } else if en_half {
                        secs += unit / 2;
                        used += 3;
                    }
                    (secs, used)
                }
                None => {
                    i += 1;
                    continue;
                }
            }
        } else {
            i += 1;
            continue;
        };
        total += secs;
        first.get_or_insert(i);
        last = i + used;
        i += used;
    }
    first.map(|f| (total, f, last))
}

/// "alle 17", "alle 17:30", "alle 5 e mezza", "at 5 pm", "at 17.30": the next such time of day.
fn clock_time(w: &[String]) -> Option<NaiveTime> {
    let at = w.iter().position(|x| matches!(x.as_str(), "alle" | "all" | "at" | "ore"))?;
    let (h, used) = number_at(w, at + 1)?;
    let j = at + 1 + used;
    let mut m = 0;
    if let Some((n, _)) = number_at(w, j).filter(|(n, _)| *n < 60) {
        m = n;
    } else if w.get(j).is_some_and(|x| x == "e") {
        if w.get(j + 1).is_some_and(|x| x.starts_with("mezz")) {
            m = 30;
        } else if w.get(j + 1).is_some_and(|x| x == "un") && w.get(j + 2).is_some_and(|x| x == "quarto") {
            m = 15;
        } else if let Some((n, _)) = number_at(w, j + 1).filter(|(n, _)| *n < 60) {
            m = n;
        }
    }
    let afternoon = w.iter().any(|x| matches!(x.as_str(), "pm" | "pomeriggio" | "sera" | "stasera"));
    let h = if afternoon && h < 12 { h + 12 } else { h };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// What a reminder is for: what's left of the sentence once the request and
/// the time are taken out ("ricordami tra 10 minuti di chiamare Marco": "chiamare Marco").
fn reminder_text(text: &str) -> Option<String> {
    let original: Vec<&str> = text.split_whitespace().collect();
    let lowered: Vec<String> = original.iter().map(|o| words(o).join(" ")).collect();
    let drop_words = [
        "ricordami", "ricorda", "ricordarmi", "ricordare", "promemoria", "remind", "reminder", "me", "mi", "un", "a", "imposta", "metti", "set",
        "per", "favore", "please", "alexa",
    ];
    let mut keep = vec![true; original.len()];
    // The time: "tra/fra/in N unit", "alle H[:M]", "at H[:M] pm", "tomorrow"...
    for i in 0..original.len() {
        let l = lowered[i].as_str();
        if matches!(l, "tra" | "fra" | "in" | "entro" | "alle" | "all" | "at") {
            let rest: Vec<String> = lowered[i + 1..].iter().flat_map(|x| x.split(' ').map(str::to_string)).collect();
            let timed = duration(&rest).is_some_and(|(_, from, _)| from == 0) || (matches!(l, "alle" | "all" | "at") && rest.first().and_then(|x| number_word(x)).is_some());
            if timed {
                keep[i] = false;
                let mut j = i + 1;
                while j < original.len() {
                    let x = lowered[j].as_str();
                    let part_of_time = number_word(x.split(' ').next().unwrap_or("")).is_some()
                        || x.split(' ').all(|p| unit_secs(p).is_some() || matches!(p, "e" | "and" | "mezzo" | "mezza" | "mezz" | "quarto" | "d" | "half" | "pm" | "am" | "del" | "di" | "pomeriggio" | "sera" | "mattina" | "ora"));
                    if !part_of_time || x.is_empty() {
                        break;
                    }
                    // "di" belongs to the time only before "pomeriggio"/"sera".
                    if matches!(x, "di" | "del") && !lowered.get(j + 1).is_some_and(|n| matches!(n.as_str(), "pomeriggio" | "sera" | "mattina")) {
                        break;
                    }
                    keep[j] = false;
                    j += 1;
                }
            }
        }
    }
    let mut out: Vec<&str> = Vec::new();
    for (i, o) in original.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        let l = lowered[i].as_str();
        // The request and the connective before the text.
        if out.is_empty() && (drop_words.contains(&l) || matches!(l, "di" | "che" | "to" | "that" | "about" | "of")) {
            continue;
        }
        out.push(o);
    }
    let text = out.join(" ");
    let text = text.trim().trim_end_matches(['.', '!', '?', ',']).trim().trim_start_matches([',', ':']).trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A timer's name in the request: "timer per la pasta" / "pasta timer".
fn timer_label(w: &[String], it: bool) -> Option<String> {
    const SKIP: &[&str] = &[
        "il", "la", "lo", "le", "i", "gli", "l", "un", "una", "a", "an", "the", "my", "set", "start", "imposta", "metti", "avvia", "fai", "new",
        "nuovo", "alexa", "hey", "ok", "hi", "esp", "please", "another", "altro",
    ];
    let not_label = |x: &str| SKIP.contains(&x) || number_word(x).is_some() || unit_secs(x).is_some() || matches!(x, "e" | "and" | "mezzo" | "mezza" | "di" | "da" | "of" | "for" | "per" | "timer");
    if let Some(p) = w.iter().position(|x| x == "per" || x == "for") {
        let rest: Vec<&String> = w[p + 1..].iter().skip_while(|x| SKIP.contains(&x.as_str())).take_while(|x| !not_label(x)).take(2).collect();
        if !rest.is_empty() {
            return Some(rest.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "));
        }
    }
    // Before the word only in English: "pasta timer".
    let t = w.iter().position(|x| x == "timer").filter(|_| !it)?;
    let before = w.get(t.checked_sub(1)?)?;
    (!not_label(before)).then(|| before.clone())
}

fn is_stop(w: &[String]) -> bool {
    has(w, &["stop", "basta", "ferma", "fermati", "silenzio", "zitto", "zitta", "smettila", "spegni", "ok", "okay", "grazie", "thanks", "enough", "quiet", "dismiss", "shut"])
}

/// "Mancano 4 minuti" / "4 minutes left", for the one ending first.
fn time_left(timers: &[TimerInfo], it: bool) -> String {
    let Some(t) = timers.iter().find(|t| t.state != "ring") else {
        return if it { "Non ci sono timer attivi." } else { "There are no timers running." }.into();
    };
    // Minutes are enough from two minutes up.
    let left = if t.left_s >= 120 { (t.left_s + 30) / 60 * 60 } else { t.left_s };
    let span = timers::span_words(left, it);
    let paused = t.state == "pause";
    match (it, t.label.as_deref(), t.kind) {
        (true, _, timers::Kind::Work) => format!("Mancano {span} alla pausa."),
        (false, _, timers::Kind::Work) => format!("{span} to the break."),
        (true, Some(l), timers::Kind::Reminder) => format!("Mancano {span} al promemoria {l}."),
        (true, Some(l), _) if paused => format!("Il timer {l} è in pausa: mancano {span}."),
        (true, Some(l), _) => format!("Al timer {l} mancano {span}."),
        (true, None, _) if paused => format!("Il timer è in pausa: mancano {span}."),
        (true, None, _) => format!("Mancano {span}."),
        (false, Some(l), _) if paused => format!("The {l} timer is paused with {span} left."),
        (false, Some(l), _) => format!("{span} left on the {l} timer."),
        (false, None, _) => format!("{span} left{}.", if paused { ", paused" } else { "" }),
    }
}

/// Timers, reminders and the pomodoro.
fn timer_plan(text: &str, w: &[String], it: bool, ctx: &Context) -> Option<Plan> {
    let failure = if it { "Non c'è nessun timer così." } else { "There's no timer like that." };
    let with_failure = |mut p: Plan| {
        p.failure = failure.into();
        p
    };
    let cancel = has(w, &["annulla", "cancella", "elimina", "togli", "rimuovi", "cancel", "delete", "remove", "clear"]) || is_stop(w);
    let all = has(w, &["tutti", "tutte", "all", "every"]);
    let said_timer = has(w, &["timer*", "countdown*", "cronometro", "sveglia"]) || has_pair(w, &[("conto", "alla")]);

    // "Stop!" while it rings.
    if ctx.alarm && is_stop(w) && w.len() <= 4 {
        return Some(with_failure(plan(vec![tool("control_timer", json!({"action": "cancel"}))], it, "Ok.".into())));
    }

    if has(w, &["pomodor*"]) {
        if cancel || has(w, &["termina", "interrompi", "finisci", "end", "finish"]) {
            let reply = if it { "Pomodoro fermato." } else { "Pomodoro stopped." };
            return Some(with_failure(plan(vec![tool("pomodoro", json!({"action": "stop"}))], it, reply.into())));
        }
        let work = duration(w).map(|(s, ..)| (s / 60).clamp(1, 120)).unwrap_or(25);
        let reply = if it { format!("Pomodoro avviato: {} di lavoro.", timers::span_words(work * 60, true)) } else { format!("Pomodoro started: {} of work.", timers::span_words(work * 60, false)) };
        return Some(plan(vec![tool("pomodoro", json!({"action": "start", "work_minutes": work}))], it, reply));
    }

    if has(w, &["ricordami", "ricorda", "ricordarmi", "promemoria", "remind*"]) {
        if cancel {
            let which = if all { "all" } else { "reminder" };
            let reply = if it { "Promemoria annullato." } else { "Reminder cancelled." };
            return Some(with_failure(plan(vec![tool("control_timer", json!({"action": "cancel", "which": which}))], it, reply.into())));
        }
        let what = reminder_text(text);
        let in_time = duration(w).filter(|&(_, from, _)| from > 0 && matches!(w[from - 1].as_str(), "tra" | "fra" | "in" | "entro"));
        let at = clock_time(w);
        let (args, due) = match (in_time, at) {
            (Some((secs, ..)), _) => (json!({"in_minutes": secs as f64 / 60.0}), Local::now() + chrono::Duration::seconds(secs as i64)),
            (None, Some(t)) => {
                let now = Local::now();
                let today = now.date_naive().and_time(t);
                let due = today.and_local_timezone(Local).earliest().filter(|d| *d > now).unwrap_or_else(|| (today + chrono::Duration::days(1)).and_local_timezone(Local).earliest().unwrap_or(now));
                (json!({"at": timers::clock_text(t)}), due)
            }
            (None, None) => {
                let reply = if it { "Quando te lo devo ricordare? Dimmi per esempio: tra 10 minuti, o alle 17." } else { "When should I remind you? Say for example: in 10 minutes, or at 5 pm." };
                return Some(plan(vec![], it, reply.into()));
            }
        };
        let Some(what) = what else {
            let reply = if it { "Cosa ti devo ricordare?" } else { "What should I remind you of?" };
            return Some(plan(vec![], it, reply.into()));
        };
        let mut args = args;
        args["text"] = json!(what);
        let at = due.format("%H:%M");
        let reply = if it { format!("Va bene, te lo ricordo alle {at}.") } else { format!("OK, I'll remind you at {at}.") };
        return Some(with_failure(plan(vec![tool("set_reminder", args)], it, reply)));
    }

    let how_long = has_pair(w, &[("quanto", "manca"), ("quanto", "tempo"), ("how", "long"), ("how", "much"), ("time", "left")]) || has(w, &["mancano", "manca"]);
    if how_long && (said_timer || !ctx.timers.is_empty()) && !has_pair(w, &[("che", "or")]) {
        return Some(plan(vec![], it, time_left(&ctx.timers, it)));
    }
    if !said_timer {
        return None;
    }
    let label = timer_label(w, it);
    let which = if all { Some("all".to_string()) } else { label.clone() };
    let which_arg = |action: &str| match &which {
        Some(w) => json!({"action": action, "which": w}),
        None => json!({"action": action}),
    };
    if has(w, &["riprendi", "continua", "riparti", "resume", "unpause", "restart"]) {
        let reply = if it { "Timer ripartito." } else { "Timer resumed." };
        return Some(with_failure(plan(vec![tool("control_timer", which_arg("resume"))], it, reply.into())));
    }
    if has(w, &["pausa", "sospendi", "pause"]) {
        let reply = if it { "Timer in pausa." } else { "Timer paused." };
        return Some(with_failure(plan(vec![tool("control_timer", which_arg("pause"))], it, reply.into())));
    }
    if cancel {
        let reply = match (it, all) {
            (true, true) => "Tutti i timer annullati.",
            (true, false) => "Timer annullato.",
            (false, true) => "All timers cancelled.",
            (false, false) => "Timer cancelled.",
        };
        return Some(with_failure(plan(vec![tool("control_timer", which_arg("cancel"))], it, reply.into())));
    }
    let Some((secs, ..)) = duration(w) else {
        if how_long || has(w, &["quanto", "how"]) {
            return Some(plan(vec![], it, time_left(&ctx.timers, it)));
        }
        let reply = if it { "Di quanto lo imposto? Dimmi per esempio: timer 10 minuti." } else { "How long for? Say for example: timer 10 minutes." };
        return Some(plan(vec![], it, reply.into()));
    };
    let span = timers::span_words(secs, it);
    let reply = match (it, &label) {
        (true, Some(l)) => format!("Timer {l} di {span} avviato."),
        (true, None) => format!("Timer di {span} avviato."),
        (false, Some(l)) => format!("{} timer set for {span}.", capitalize(l)),
        (false, None) => format!("Timer set for {span}."),
    };
    let mut args = json!({"seconds": secs});
    if let Some(l) = &label {
        args["label"] = json!(l);
    }
    Some(plan(vec![tool("set_timer", args)], it, reply))
}

/// "Apri Spotify", "puoi aprire la calcolatrice", "open Safari", "launch
/// Visual Studio Code": open_app with the name. "Avvia la musica" stays the
/// music's, and "start a timer" was the timer's already.
fn app_plan(w: &[String], it: bool) -> Option<Plan> {
    const OPEN: &[&str] = &["apri", "aprimi", "aprire", "open"];
    const START: &[&str] = &["avvia", "avviare", "lancia", "lanciare", "esegui", "launch", "start", "run"];
    // Near the start: "Alexa, apri...", "can you open...", "potresti aprire...".
    let verb = w.iter().take(4).position(|x| OPEN.contains(&x.as_str()) || START.contains(&x.as_str()))?;
    const FILLER: &[&str] = &[
        "l", "la", "il", "lo", "le", "un", "una", "mi", "me", "my", "the", "up", "app", "applicazione", "programma", "application",
        "program", "di", "per", "favore", "please",
    ];
    let mut name: Vec<&str> = w[verb + 1..].iter().map(String::as_str).skip_while(|x| FILLER.contains(x)).collect();
    while name.last().is_some_and(|x| matches!(*x, "please" | "grazie" | "favore" | "per" | "thanks" | "app" | "application" | "applicazione" | "program" | "programma")) {
        name.pop();
    }
    let started = START.contains(&w[verb].as_str());
    let generic = |x: &str| has(&[x.to_string()], MUSIC_WORDS) && x != "spotify" || has(&[x.to_string()], &["timer*", "pomodor*", "countdown*", "cronometro"]);
    if started && (name.is_empty() || name.iter().any(|x| generic(x))) {
        return None;
    }
    if name.is_empty() {
        return Some(plan(vec![], it, if it { "Quale app apro?" } else { "Which app should I open?" }.into()));
    }
    let said = name.join(" ");
    let shown = said.split(' ').map(capitalize).collect::<Vec<_>>().join(" ");
    let mut p = plan(vec![tool("open_app", json!({"app": said}))], it, if it { format!("Apro {shown}.") } else { format!("Opening {shown}.") });
    p.failure = if it { format!("Non trovo l'app {shown}.") } else { format!("I can't find the app {shown}.") };
    Some(p)
}

const MUSIC_WORDS: &[&str] = &["music*", "musica", "canzon*", "brano", "brani", "song*", "track*", "traccia", "spotify", "pezzo"];

/// Words about the music playing, so the voice looks at what plays first.
pub fn asks_music(text: &str) -> bool {
    let w = words(text);
    has(&w, MUSIC_WORDS) || has(&w, &["suonando", "playing", "ascoltando", "listening", "pausa", "pause", "skip", "salta"])
}

/// "Metti in pausa la musica", "prossima canzone", "next song", "cosa sta
/// suonando": media_control, or the answer from what plays.
fn music_plan(w: &[String], it: bool, ctx: &Context) -> Option<Plan> {
    // A face: "metti la faccia musica".
    if has(w, &["faccia", "face", "quadrante"]) {
        return None;
    }
    let about = has(w, MUSIC_WORDS);
    let asks = has(w, &["suonando", "playing", "ascoltando", "listening", "canta", "sings", "singing"])
        && (has(w, &["cosa", "che", "what", "chi", "who", "quale", "which"]) || about);
    if asks {
        let reply = match (&ctx.music, it) {
            (Some(n), true) if !n.artist.is_empty() => format!("{} di {}.", n.title, n.artist),
            (Some(n), false) if !n.artist.is_empty() => format!("{} by {}.", n.title, n.artist),
            (Some(n), _) => format!("{}.", n.title),
            (None, true) => "Non sta suonando niente.".into(),
            (None, false) => "Nothing is playing.".into(),
        };
        return Some(plan(vec![], it, reply));
    }
    // Without a word about music, "pausa" alone is for it only while it plays.
    let playing = ctx.music.as_ref().is_some_and(|m| m.playing);
    if !about && !(playing && w.len() <= 3) && !has(w, &["skip", "salta"]) {
        return None;
    }
    let (action, reply) = if has(w, &["prossim*", "successiv*", "next", "skip", "salta", "avanti"]) {
        ("next", if it { "Prossima canzone." } else { "Next song." })
    } else if has(w, &["precedent*", "previous", "indietro", "back", "prima"]) {
        ("previous", if it { "Canzone precedente." } else { "Previous song." })
    } else if has(w, &["pausa", "pause", "ferma", "fermala", "stop", "stoppa", "basta"]) {
        ("pause", if it { "In pausa." } else { "Paused." })
    } else if has(w, &["riprendi", "play", "resume", "continua", "suona", "riproduci", "fai", "metti", "avvia", "start"]) {
        ("play", if it { "Ecco la musica." } else { "Playing." })
    } else {
        return None;
    };
    let mut p = plan(vec![tool("media_control", json!({"action": action}))], it, reply.into());
    p.failure = if it { "Non trovo nessun lettore musicale aperto.".into() } else { "I can't find a music player open.".into() };
    Some(p)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Whether `text` asks about the sensors ("is the CPU hot?", "quanto lavora
/// la GPU?") rather than to change something.
pub fn asks_metrics(text: &str) -> bool {
    let w = words(text);
    let about = has(&w, &[
        "temperatur*", "caldo", "calda", "caldi", "gradi", "hot", "warm", "busy", "load", "carico", "carica", "lavora*", "working", "fan", "fans",
        "ventol*", "rpm", "watt", "consum*", "power", "clock", "frequenz*", "memoria", "ram", "vram",
    ]);
    let command = has(&w, &["metti", "mostra", "imposta", "cambia", "put", "show", "set", "switch", "faccia", "face", "ruota", "rotate", "scrivi", "write", "timer*"]);
    about && !command
}

/// Whether `text` asks the time, the day or the date.
pub fn asks_time(text: &str) -> bool {
    let w = words(text);
    has(&w, &["orario", "clock"])
        || has_pair(&w, &[("che", "or"), ("che", "giorn"), ("che", "data"), ("what", "time"), ("the", "time"), ("what", "day"), ("what", "date"), ("the", "date"), ("e", "oggi")])
}

/// `it` or `en`, whichever `text` looks more like (for text not heard but
/// typed, which Whisper didn't label).
pub fn guess_language(text: &str) -> &'static str {
    const IT: &[&str] = &[
        "il", "lo", "la", "gli", "le", "di", "che", "e", "un", "una", "per", "non", "sono", "ciao", "questo", "come", "sei", "ho", "del",
        "della", "con", "sullo", "sulla", "sui", "al", "alla", "ai", "gradi", "fatto", "ecco", "schermo", "schermi", "faccia", "posso",
        "ti", "mi", "si", "anche", "ancora", "piu", "ora", "adesso", "sinistra", "destra", "sinistro", "destro", "entrambi",
    ];
    const EN: &[&str] = &[
        // Not "a" or "i": Italian has them too.
        "the", "an", "of", "and", "is", "are", "to", "you", "it", "this", "hello", "what", "in", "on", "with", "for", "not", "i'm",
        "done", "screen", "screens", "degrees", "turned", "set", "can", "your", "my", "here", "now", "left", "right", "both", "sorry",
    ];
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
    fn time_questions() {
        for q in ["Che ore sono?", "Che giorno è oggi?", "What time is it?", "What's the date today?", "Tell me the time"] {
            assert!(asks_time(q), "{q}");
        }
        for q in ["Ora metti rings a sinistra", "Put rings on the left", "Quanto è calda la CPU?"] {
            assert!(!asks_time(q), "{q}");
        }
    }

    #[test]
    fn timers_by_rules() {
        assert_eq!(calls("Alexa, timer 10 minuti", "it"), [tool("set_timer", json!({"seconds": 600}))]);
        assert_eq!(calls("Imposta un timer di un'ora e mezza", "it"), [tool("set_timer", json!({"seconds": 5400}))]);
        assert_eq!(calls("Metti un timer di mezz'ora per la pasta", "it"), [tool("set_timer", json!({"seconds": 1800, "label": "pasta"}))]);
        assert_eq!(calls("Timer di venticinque minuti", "it"), [tool("set_timer", json!({"seconds": 1500}))]);
        assert_eq!(calls("Un timer di un minuto e mezzo", "it"), [tool("set_timer", json!({"seconds": 90}))]);
        assert_eq!(calls("Set a pasta timer for 8 minutes", "en"), [tool("set_timer", json!({"seconds": 480, "label": "pasta"}))]);
        assert_eq!(calls("Set a timer for twenty five minutes", "en"), [tool("set_timer", json!({"seconds": 1500}))]);
        assert_eq!(calls("Start a timer for two and a half minutes", "en"), [tool("set_timer", json!({"seconds": 150}))]);
        assert_eq!(calls("Timer 1 ora e 30 minuti", "it"), [tool("set_timer", json!({"seconds": 5400}))]);
        let p = understand("Timer 10 minuti", "it", &Context::default());
        assert_eq!(p.reply, "Timer di 10 minuti avviato.");
        assert_eq!(calls("Annulla il timer", "it"), [tool("control_timer", json!({"action": "cancel"}))]);
        assert_eq!(calls("Cancel all timers", "en"), [tool("control_timer", json!({"action": "cancel", "which": "all"}))]);
        assert_eq!(calls("Metti in pausa il timer", "it"), [tool("control_timer", json!({"action": "pause"}))]);
        assert_eq!(calls("Riprendi il timer della pasta", "it")[0].1["action"], "resume");
        let ringing = Context { alarm: true, ..Default::default() };
        assert_eq!(understand("Stop!", "en", &ringing).calls, [tool("control_timer", json!({"action": "cancel"}))]);
        assert!(understand("Stop!", "en", &Context::default()).calls.is_empty());
    }

    #[test]
    fn reminders_and_pomodoro_by_rules() {
        let c = calls("Ricordami tra 20 minuti di chiamare Marco", "it");
        assert_eq!(c, [tool("set_reminder", json!({"in_minutes": 20.0, "text": "chiamare Marco"}))]);
        let c = calls("Ricordami di chiamare Marco tra 20 minuti.", "it");
        assert_eq!(c[0].1["text"], "chiamare Marco");
        let c = calls("Remind me to stretch at 5 pm", "en");
        assert_eq!(c, [tool("set_reminder", json!({"at": "17:00", "text": "stretch"}))]);
        let c = calls("Ricordami alle 17 e 30 di uscire", "it");
        assert_eq!(c, [tool("set_reminder", json!({"at": "17:30", "text": "uscire"}))]);
        assert!(calls("Ricordami di bere", "it").is_empty());
        assert_eq!(calls("Avvia un pomodoro", "it"), [tool("pomodoro", json!({"action": "start", "work_minutes": 25}))]);
        assert_eq!(calls("Start a 50 minute pomodoro", "en"), [tool("pomodoro", json!({"action": "start", "work_minutes": 50}))]);
        assert_eq!(calls("Ferma il pomodoro", "it"), [tool("pomodoro", json!({"action": "stop"}))]);
    }

    #[test]
    fn time_left_on_timers() {
        let t = TimerInfo { id: 1, kind: timers::Kind::Timer, label: Some("pasta".into()), total_s: 600, left_s: 272, state: "run".into(), ends_at: None };
        let ctx = Context { timers: vec![t], ..Default::default() };
        assert_eq!(understand("Quanto manca al timer?", "it", &ctx).reply, "Al timer pasta mancano 5 minuti.");
        assert_eq!(understand("How long is left?", "en", &ctx).reply, "5 minutes left on the pasta timer.");
        assert_eq!(understand("Quanto manca?", "it", &Context::default()).reply, "Non ho capito.");
        assert_eq!(number_word("ventuno"), Some(21));
        assert_eq!(number_word("trentotto"), Some(38));
        assert_eq!(number_word("quarantacinque"), Some(45));
        assert_eq!(number_word("sei"), Some(6));
    }

    #[test]
    fn music_by_rules() {
        let media = |a: &str| vec![tool("media_control", json!({"action": a}))];
        assert_eq!(calls("Metti in pausa la musica", "it"), media("pause"));
        assert_eq!(calls("Prossima canzone", "it"), media("next"));
        assert_eq!(calls("Next song please", "en"), media("next"));
        assert_eq!(calls("Play the previous track", "en"), media("previous"));
        assert_eq!(calls("Riprendi la musica", "it"), media("play"));
        assert_eq!(calls("Skip", "en"), media("next"));
        // The timer's pause stays the timer's; a face stays a face.
        assert_eq!(calls("Metti in pausa il timer", "it"), [tool("control_timer", json!({"action": "pause"}))]);
        assert_eq!(calls("Metti la faccia musica a destra", "it"), [tool("set_face", json!({"face": "music", "screen": "right"}))]);
        assert_eq!(calls("Show the eyes on both screens", "en"), [tool("set_face", json!({"face": "eyes", "screen": "both"}))]);
        // "Pausa" alone is the music's only while it plays.
        assert!(!understand("Pausa", "it", &Context::default()).understood);
        let track = crate::music::tests_track();
        let playing = Context { music: Some(track), ..Context::default() };
        assert_eq!(understand("Pausa", "it", &playing).calls, media("pause"));
        let asked = understand("Cosa sta suonando?", "it", &playing);
        assert!(asked.calls.is_empty());
        assert_eq!(asked.reply, "Zitti e buoni di Måneskin.");
        assert_eq!(understand("What's playing?", "en", &Context::default()).reply, "Nothing is playing.");
        assert!(asks_music("che canzone è questa"));
    }

    #[test]
    fn apps_by_rules() {
        let open = |n: &str| vec![tool("open_app", json!({"app": n}))];
        assert_eq!(calls("Apri Spotify", "it"), open("spotify"));
        assert_eq!(calls("Alexa, apri la calcolatrice per favore", "it"), open("calcolatrice"));
        assert_eq!(calls("Puoi aprire Visual Studio Code?", "it"), open("visual studio code"));
        assert_eq!(calls("Open the Safari app", "en"), open("safari"));
        assert_eq!(calls("Launch Google Chrome please", "en"), open("google chrome"));
        assert_eq!(calls("Avvia Spotify", "it"), open("spotify"));
        assert_eq!(understand("Apri Spotify", "it", &Context::default()).reply, "Apro Spotify.");
        // The music's and the timers' own starts stay theirs.
        assert_eq!(calls("Avvia la musica", "it"), [tool("media_control", json!({"action": "play"}))]);
        assert_eq!(calls("Start a timer for five minutes", "en"), [tool("set_timer", json!({"seconds": 300}))]);
        assert_eq!(calls("Avvia un pomodoro", "it")[0].0, "pomodoro");
        let p = understand("Apri", "it", &Context::default());
        assert!(p.calls.is_empty() && p.understood);
    }

    #[test]
    fn sensor_questions() {
        for q in ["Is the CPU running hot?", "How busy is the graphics card?", "A quanti gradi è la GPU?", "Quanto stanno girando le ventole?"] {
            assert!(asks_metrics(q), "{q}");
        }
        for q in ["Metti la faccia rings a sinistra", "Show the fans face", "Timer 10 minuti", "Che ore sono?"] {
            assert!(!asks_metrics(q), "{q}");
        }
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
