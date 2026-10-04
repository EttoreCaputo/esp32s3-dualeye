// What to paste into ElevenLabs' Voice Design (elevenlabs.io → Voices → Voice design)
// to make a voice for each personality. Written the way its guide asks: language,
// who, quality, persona, emotion, then timbre and pacing; each with lines of its own
// (100–1,000 characters) for the previews. Clear diction first: the board's speaker
// is small.
import type { Personality } from "./monitor.svelte";

export type VoicePrompt = {
  /** A name to save it as. */
  name: string;
  /** The Prompt field. */
  description: string;
  /** The Text to preview field. */
  sample: string;
};

export const VOICE_PROMPTS: Partial<Record<Personality, VoicePrompt>> = {
  cute: {
    name: "Micio",
    description:
      "Native Italian, standard neutral Italian pronunciation. Young adult female, early 20s, a small, light voice with a naturally high pitch. " +
      "Studio quality. Persona: affectionate desk companion cat. Emotion: warm, cheerful, tender. " +
      "Soft rounded timbre with a smile always audible, a gentle lift at the end of phrases; medium pace and crisp, careful diction so every word stays clear on a tiny speaker.",
    sample:
      "Ciao! Sono qui, sulla tua scrivania, e tengo d'occhio tutto io. La CPU è a quarantadue gradi, quindi va tutto benissimo. " +
      "Ho messo un timer di dieci minuti per la pasta: quando suona ti chiamo, promesso. E se ti serve una pausa, dimmelo pure!",
  },
  playful: {
    name: "Birba",
    description:
      "Native Italian, standard neutral Italian pronunciation. Young adult, androgynous, a bright and nimble voice with a high-mid pitch. " +
      "Studio quality. Persona: mischievous cartoon cat sidekick. Emotion: cheeky, energetic, amused. " +
      "Springy, playful intonation with quick rises and a light, slightly nasal edge; lively fast pace with a grin in the voice, yet every syllable clearly articulated.",
    sample:
      "Eccomi! Allora, vediamo un po': la scheda video è a settantotto gradi, sta sudando più di te in palestra! " +
      "Vuoi che metta un po' di musica? Ho già in mente la canzone giusta. E no, stavolta non ti dico quanto manca alla riunione, era una sorpresa!",
  },
  calm: {
    name: "Fusa",
    description:
      "Native Italian, standard neutral Italian pronunciation. Adult female, 30s, a low-mid pitch, velvety and hushed. " +
      "Studio quality. Persona: calm late-night companion. Emotion: serene, reassuring, unhurried. " +
      "Close and intimate, soft onsets and a faint warm breathiness; slow, relaxed pace with gentle pauses, never sleepy, consonants still clean and clear.",
    sample:
      "Va tutto bene. Il computer è tranquillo, la temperatura è bassa e non c'è niente di cui preoccuparsi. " +
      "Ho abbassato la luminosità degli schermi, così gli occhi riposano un po'. Prenditi il tuo tempo: io resto qui, in silenzio, finché non mi chiami.",
  },
  sassy: {
    name: "Sornione",
    description:
      "Native Italian, standard neutral Italian pronunciation. Adult male, 30s, a mid pitch, smooth with a slightly husky texture. " +
      "Studio quality. Persona: sly, witty house cat. Emotion: dry, amused, self-assured. " +
      "Laid-back, half-smiling delivery with deadpan timing and knowing little pauses; relaxed pace, precise and polished diction.",
    sample:
      "Oh, eccoti. Il processore è al novanta per cento da mezz'ora, ma immagino che quelle quaranta schede aperte siano tutte indispensabili. " +
      "D'accordo, ho cambiato il quadrante come volevi. Non ringraziarmi, davvero: mi basta essere apprezzato in silenzio.",
  },
  butler: {
    name: "Maggiordomo",
    description:
      "Native Italian, standard neutral Italian pronunciation. Mature male, 50s, a low-mid pitch, rich and resonant. " +
      "Studio quality. Persona: refined household butler. Emotion: composed, courteous, discreet. " +
      "Elegant, measured pace with crisp formal articulation and a warm, restrained resonance; the faint hint of a smile, never stiff.",
    sample:
      "Buonasera. Mi permetta di informarla che la temperatura del processore è di cinquantacinque gradi, del tutto nella norma. " +
      "Ho predisposto un promemoria per le diciotto e trenta, come da sua richiesta. Desidera che abbassi il volume della musica?",
  },
  minimal: {
    name: "Essenziale",
    description:
      "Native Italian, standard neutral Italian pronunciation. Adult female, 30s, a mid pitch, clear and neutral. " +
      "Broadcast quality. Persona: precise onboard assistant. Emotion: calm, confident, efficient. " +
      "Even, steady intonation with clean, crisp consonants and a brisk moderate pace; no theatrical flourish, like a high-end car announcing its status.",
    sample:
      "Processore: quarantotto gradi, carico al dodici per cento. Scheda video: sessantuno gradi. " +
      "Timer impostato: venticinque minuti. Volume al sessanta per cento. Musica in pausa. Quadrante classico su entrambi gli schermi. Nessun avviso.",
  },
};
