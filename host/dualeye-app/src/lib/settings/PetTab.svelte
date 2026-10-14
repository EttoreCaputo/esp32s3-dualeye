<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { monitor, type QuipLevel } from "../monitor.svelte";
  import PetGallery from "../PetGallery.svelte";
  import PetMood from "../PetMood.svelte";
  import { settingsData } from "./data.svelte";
  import NeedRow from "./NeedRow.svelte";
  import { need } from "./requirements";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  const tp = $derived(i18n.t.settings.pet);
  const tc = $derived(i18n.t.settings.common);
  // The pet's mood: the monitor reads it every few seconds.
  const pet = $derived(monitor.pet);
  const quips = $derived(settingsData.quips);
  const connected = $derived(monitor.link === "connected");
  let error = $state("");

  $effect(() => {
    const id = setInterval(() => settingsData.loadQuips().catch(() => {}), 5000);
    return () => clearInterval(id);
  });

  // Whether the eyes play while idle lives on the board.
  let idleEyes = $state<boolean | null>(null);
  $effect(() => {
    if (!connected) return;
    let alive = true;
    monitor
      .boardVoice()
      .then((b) => alive && (idleEyes = b.idle_eyes))
      .catch(() => alive && (idleEyes = null));
    return () => {
      alive = false;
    };
  });

  async function setIdleEyes(on: boolean) {
    idleEyes = on;
    try {
      await monitor.setBoardIdleEyes(on);
    } catch (e) {
      idleEyes = !on;
      error = String(e);
    }
  }

  async function setPet(key: "sounds" | "reactions" | "hearing", on: boolean) {
    if (!monitor.pet) return;
    monitor.pet[key] = on;
    error = "";
    try {
      await monitor.setBoardPet(key === "sounds" ? { sounds: on } : key === "reactions" ? { react: on } : { hear: on });
    } catch (e) {
      if (monitor.pet) monitor.pet[key] = !on;
      error = String(e);
    }
  }

  const QUIP_LEVELS: QuipLevel[] = ["off", "rare", "often"];
  async function setQuips(level: QuipLevel) {
    error = "";
    try {
      settingsData.quips = await monitor.setQuips(level);
    } catch (e) {
      error = String(e);
    }
  }
</script>

{#if pet}<div class="own-styles"><PetMood {pet} /></div>{/if}
<Section class="petset" anchor="pet-behaviour" title={tp.behaviour} tip={tp.behaviourTip}>
  <Toggle anchor="idle-eyes" checked={idleEyes ?? true} disabled={idleEyes === null} title={idleEyes === null ? tc.needsFirmware("1.0.2") : ""} label={tp.idleEyes} onchange={setIdleEyes} />
  <Toggle anchor="reactions" checked={pet?.reactions ?? true} disabled={!pet} title={!pet ? tc.needsFirmware("1.4.0") : ""} label={tp.reactions} tip={tp.reactionsTip} onchange={(on) => setPet("reactions", on)} />
  <Toggle anchor="pet-sounds" checked={pet?.sounds ?? true} disabled={!pet} title={!pet ? tc.needsFirmware("1.4.0") : ""} label={tp.sounds} tip={tp.soundsTip} onchange={(on) => setPet("sounds", on)} />
  <Toggle
    anchor="hearing"
    checked={pet?.hearing ?? true}
    disabled={pet?.hearing === undefined}
    title={pet?.hearing === undefined ? tc.needsFirmware("1.4.2") : ""}
    label={tp.hearing}
    tip={tp.hearingTip}
    onchange={(on) => setPet("hearing", on)}
  />
</Section>
<Section class="petset" anchor="quips" title={tp.quips} tip={tp.quipsTip}>
  <div class="rotation">
    <span class="rlabel">{tp.howOften}</span>
    <div class="rots three" role="radiogroup" aria-label={tp.howOftenAria}>
      {#each QUIP_LEVELS as level (level)}
        {@const [label, title] = tp.levels[level]}
        <button class="rot" class:checked={quips?.level === level} role="radio" aria-checked={quips?.level === level} {title} disabled={!quips} onclick={() => setQuips(level)}>{label}</button>
      {/each}
    </div>
  </div>
  {#if quips && quips.level !== "off"}<NeedRow need={need("speak")} />{/if}
  {#if quips?.last}
    <p class="hint">{tp.last(i18n.time(quips.last.at), quips.last.text)}{quips.last.error ? tp.notSaid(quips.last.error) : ""}</p>
  {/if}
  {#if error}<p class="hint error">{error}</p>{/if}
</Section>
<div class="own-styles"><PetGallery {connected} petSounds={pet?.sounds ?? true} oldFirmware={connected && pet === null} /></div>
