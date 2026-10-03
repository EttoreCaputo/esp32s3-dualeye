<!--
  The watch faces of one screen, picked from the mirror: opens under the
  screen that was clicked, with the options of the face it's on.
-->
<script lang="ts">
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Eye from "./Eye.svelte";
  import { DEVICES, FACES, ROTATIONS, hasSource, screenFor, type DeviceId, type Face, type Rotation, type Source } from "./firmware";
  import { fanRpm, monitor, type Side } from "./monitor.svelte";

  let { id, anchor, onclose }: { id: DeviceId; anchor: { x: number; y: number }; onclose: () => void } = $props();

  const side: Side = $derived(id === "cpu" ? "left" : "right");
  const other: DeviceId = $derived(id === "cpu" ? "gpu" : "cpu");
  const current = $derived(monitor.faces[id]);
  const src = $derived(monitor.faces.src[id]);

  const GROUPS: [string, Face[]][] = [
    ["This computer", ["classic", "rings", "plus", "bar", "net", "disk", "battery"]],
    ["Claude Code", ["claude", "clawd"]],
    ["Fun & tools", ["music", "timer", "eyes", "image"]],
  ];
  const face = (f: Face) => FACES.find((x) => x.id === f)!;

  // Thumbnails use the host's latest sample, so they have data even with no board attached.
  const preview = (f: Face) => {
    const s = monitor.faces.src[id];
    const last = monitor.last;
    return screenFor(s, f, last?.[s], false, false, {
      fan: fanRpm(last, s),
      claude: last?.claude,
      net: last?.net,
      disk: last?.disk,
      bat: last?.bat,
      image: monitor.images[side],
      // A sample one, so the thumbnail shows what the face looks like.
      timer: last?.timer ?? { kind: "timer", state: "run", left_s: 272, total_s: 600 },
      music: last?.music ?? { state: "play", title: "Nothing playing", pos_s: 70, dur_s: 200 },
      cover: monitor.cover,
    });
  };

  const pick = (f: Face) => monitor.setFaces({ ...monitor.faces, [id]: f });
  const showSource = (s: Source) => monitor.setFaces({ ...monitor.faces, src: { ...monitor.faces.src, [id]: s } });
  const turn = (deg: Rotation) => monitor.setRotation({ ...monitor.rotation, [id]: deg });
  /** The other screen gets this one's face (and source). */
  const mirror = () => monitor.setFaces({ ...monitor.faces, [other]: current, src: { ...monitor.faces.src, [other]: src } });
  const same = $derived(monitor.faces[other] === current && (!hasSource(current) || monitor.faces.src[other] === src));

  let imageError = $state("");
  let imageNote = $state("");
  async function chooseImage(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    imageError = "";
    imageNote = "";
    try {
      const sent = await monitor.sendImage(side, file);
      imageNote =
        sent.frames === 1
          ? `Sent, ${Math.round(sent.bytes / 1024)} KB.`
          : `Sent ${sent.frames} frames${sent.frames < sent.source_frames ? ` of ${sent.source_frames}` : ""}, ${(sent.duration_ms / 1000).toFixed(1)} s a loop, ${Math.round(sent.bytes / 1024)} KB.`;
    } catch (err) {
      imageError = String(err);
    }
  }
  async function removeImage() {
    imageError = "";
    imageNote = "";
    try {
      await monitor.clearImage(side);
    } catch (err) {
      imageError = String(err);
    }
  }

  // Under the screen, kept inside the window.
  const W = 520;
  let winW = $state(innerWidth);
  let winH = $state(innerHeight);
  const left = $derived(Math.round(Math.min(Math.max(12, anchor.x - W / 2), winW - W - 12)));
  const top = $derived(Math.round(Math.min(anchor.y + 14, winH - 260)));
  const maxH = $derived(winH - top - 12);
  const caret = $derived(Math.round(Math.min(Math.max(24, anchor.x - left), W - 24)));

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window bind:innerWidth={winW} bind:innerHeight={winH} onkeydown={onKey} />

<button class="catcher" aria-label="Close the faces" onclick={onclose}></button>
<div
  class="picker"
  role="dialog"
  aria-label="{side === 'left' ? 'Left' : 'Right'} screen"
  style:left="{left}px"
  style:top="{top}px"
  style:max-height="{maxH}px"
  style:--caret="{caret}px"
  style:--accent={DEVICES[src].accent}
  transition:fly={{ y: -8, duration: 220, easing: cubicOut }}
>
  <header>
    <div>
      <h2>{side === "left" ? "Left" : "Right"} screen</h2>
      <p>{face(current).blurb}</p>
    </div>
    <button class="close" onclick={onclose} aria-label="Close">
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
    </button>
  </header>

  <div class="body">
    {#each GROUPS as [title, faces] (title)}
      <h3>{title}</h3>
      <div class="faces" role="radiogroup" aria-label={title}>
        {#each faces as f (f)}
          <button class="face" class:checked={current === f} role="radio" aria-checked={current === f} title={face(f).blurb} onclick={() => pick(f)}>
            <span class="thumb"><Eye {id} screen={preview(f)} board="live" size={50} /></span>
            <span class="fname">{face(f).name}</span>
          </button>
        {/each}
      </div>
    {/each}

    {#if hasSource(current) || current === "music" || current === "eyes" || current === "image"}
      <div class="options">
        {#if hasSource(current)}
          <div class="row">
            <span class="label">Shows</span>
            <div class="seg" role="radiogroup" aria-label="Shows">
              {#each ["cpu", "gpu"] as const as s (s)}
                <button class:checked={src === s} role="radio" aria-checked={src === s} onclick={() => showSource(s)}>{DEVICES[s].title}</button>
              {/each}
            </div>
          </div>
        {/if}
        {#if current === "music"}
          {@const playing = monitor.last?.music}
          <div class="row">
            <span class="label ellipsis">{playing ? `${playing.title}${playing.artist ? ` · ${playing.artist}` : ""}` : "Nothing playing"}</span>
            <div class="seg">
              <button title="Previous track" onclick={() => monitor.musicControl("previous").catch(() => {})}>⏮</button>
              <button title="Play or pause" onclick={() => monitor.musicControl("toggle").catch(() => {})}>{playing?.state === "play" ? "Pause" : "Play"}</button>
              <button title="Next track" onclick={() => monitor.musicControl("next").catch(() => {})}>⏭</button>
            </div>
          </div>
        {/if}
        {#if current === "eyes"}
          <label class="row check">
            <span class="label">Follow the mouse pointer</span>
            <input type="checkbox" checked={monitor.followPointer} onchange={(e) => monitor.setFollowPointer(e.currentTarget.checked)} />
            <span class="track"><span class="knob"></span></span>
          </label>
        {/if}
        {#if current === "image"}
          {@const sending = monitor.sending?.side === side ? monitor.sending : null}
          {@const offline = monitor.link !== "connected"}
          <div class="row">
            <span class="label">{offline ? "Connect the board to send a picture" : "PNG, JPEG, WebP or GIF"}</span>
            <div class="seg">
              <label class="file" class:disabled={monitor.sending !== null || offline}>
                {sending ? `Sending ${Math.round(sending.progress * 100)}%` : monitor.images[side] ? "Replace…" : "Choose…"}
                <input type="file" accept="image/png,image/jpeg,image/gif,image/webp,image/bmp" disabled={monitor.sending !== null || offline} onchange={chooseImage} />
              </label>
              {#if monitor.images[side]}
                <button disabled={monitor.sending !== null || offline} onclick={removeImage}>Remove</button>
              {/if}
            </div>
          </div>
          {#if imageNote}<p class="note">{imageNote}</p>{/if}
          {#if imageError}<p class="note error">{imageError}</p>{/if}
        {/if}
      </div>
    {/if}
  </div>

  <footer>
    <div class="seg" role="radiogroup" aria-label="Rotation">
      {#each ROTATIONS as deg (deg)}
        <button
          class:checked={monitor.rotation[id] === deg}
          role="radio"
          aria-checked={monitor.rotation[id] === deg}
          title={deg === 0 ? "Upright" : `Turned ${deg}° clockwise`}
          onclick={() => turn(deg)}
        >
          <svg viewBox="0 0 16 16" aria-hidden="true" style:rotate="{deg}deg">
            <circle cx="8" cy="8" r="6.2" fill="none" stroke="currentColor" stroke-width="1.3" opacity="0.45" />
            <path d="M8 3.2 L10.4 6.6 H5.6 Z" fill="currentColor" />
          </svg>
          {deg}°
        </button>
      {/each}
    </div>
    <button class="both" disabled={same} onclick={mirror}>{same ? "Same on both screens" : `Use on the ${side === "left" ? "right" : "left"} too`}</button>
  </footer>
</div>

<style>
  .catcher {
    position: fixed;
    inset: 0;
    z-index: 30;
    border: 0;
    background: transparent;
    cursor: default;
  }
  .picker {
    position: fixed;
    z-index: 31;
    width: 520px;
    max-width: calc(100vw - 24px);
    display: flex;
    flex-direction: column;
    border-radius: 18px;
    background: rgba(17, 18, 21, 0.9);
    backdrop-filter: blur(28px) saturate(1.4);
    border: 1px solid rgba(255, 255, 255, 0.09);
    box-shadow:
      0 30px 70px -18px rgba(0, 0, 0, 0.85),
      inset 0 1px 0 rgba(255, 255, 255, 0.06);
  }
  /* Points at the screen it belongs to. */
  .picker::before {
    content: "";
    position: absolute;
    top: -6px;
    left: calc(var(--caret) - 6px);
    width: 12px;
    height: 12px;
    rotate: 45deg;
    background: rgb(22, 23, 27);
    border-left: 1px solid rgba(255, 255, 255, 0.09);
    border-top: 1px solid rgba(255, 255, 255, 0.09);
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 14px 4px 16px;
  }
  h2 {
    margin: 0 0 4px;
    font: 600 13.5px/1.2 var(--sans);
  }
  header p {
    margin: 0;
    font: 400 12px/1.4 var(--sans);
    color: var(--dim);
  }
  .close {
    flex: none;
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.05);
    color: var(--dim);
    cursor: pointer;
  }
  .close:hover {
    color: var(--text);
    background: rgba(255, 255, 255, 0.09);
  }
  .close svg {
    width: 12px;
    height: 12px;
  }

  .body {
    overflow-y: auto;
    padding: 4px 16px 12px;
    min-height: 0;
  }
  h3 {
    margin: 12px 0 7px;
    font: 600 9.5px/1 var(--mono);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--faint);
  }
  .faces {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    gap: 6px;
  }
  .face {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 8px 0 7px;
    border-radius: 11px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
    color: var(--dim);
    cursor: pointer;
    transition:
      border-color 180ms,
      background 180ms,
      color 180ms,
      transform 180ms;
  }
  .face:hover {
    background: rgba(255, 255, 255, 0.05);
    transform: translateY(-1px);
  }
  .face.checked {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    color: var(--text);
  }
  .face:focus-visible {
    outline: 1.5px solid var(--accent);
  }
  .thumb {
    border-radius: 50%;
    line-height: 0;
    box-shadow:
      0 0 0 1px #000,
      0 0 0 2px rgba(255, 255, 255, 0.08);
  }
  .fname {
    font: 550 10.5px/1 var(--sans);
    white-space: nowrap;
  }
  @media (max-width: 560px) {
    .faces {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
  }

  .options {
    display: grid;
    gap: 8px;
    margin-top: 14px;
    padding: 10px 12px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .label {
    font: 500 12px/1.3 var(--sans);
    color: var(--dim);
    min-width: 0;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    margin: 0;
    font: 400 11.5px/1.4 var(--sans);
    color: var(--faint);
  }
  .note.error {
    color: var(--hot);
  }

  .seg {
    flex: none;
    display: flex;
    gap: 3px;
    padding: 3px;
    border-radius: 10px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
  }
  .seg button,
  .seg .file {
    position: relative;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 5px 9px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--dim);
    font: 550 11px/1 var(--mono);
    cursor: pointer;
    transition:
      background 180ms,
      color 180ms;
  }
  .seg button:hover:not(:disabled),
  .seg .file:hover {
    color: var(--text);
    background: rgba(255, 255, 255, 0.05);
  }
  .seg button.checked {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--text);
  }
  .seg button:disabled,
  .seg .file.disabled {
    opacity: 0.4;
    pointer-events: none;
  }
  .seg svg {
    width: 12px;
    height: 12px;
  }
  .file input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }

  .check {
    cursor: pointer;
  }
  .check input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .track {
    flex: none;
    position: relative;
    width: 32px;
    height: 18px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.1);
    transition: background 200ms;
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--dim);
    transition:
      transform 200ms,
      background 200ms;
  }
  .check input:checked + .track {
    background: color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .check input:checked + .track .knob {
    transform: translateX(14px);
    background: var(--accent);
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 14px 12px;
    border-top: 1px solid var(--line);
  }
  .both {
    height: 28px;
    padding: 0 12px;
    border-radius: 9px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.05);
    color: var(--text);
    font: 550 11.5px/1 var(--sans);
    cursor: pointer;
    white-space: nowrap;
  }
  .both:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.09);
  }
  .both:disabled {
    color: var(--faint);
    background: none;
    cursor: default;
  }
</style>
