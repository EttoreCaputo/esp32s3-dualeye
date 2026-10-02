<!--
  The board's voice eyes on one mirrored screen, drawn like main/ui_eyes.c.
  Transparent while they're shut, so the face shows through.
-->
<script lang="ts">
  import { drawEye, eyes } from "./eyes";
  import type { VoiceState } from "./monitor.svelte";

  let { eye, size, voice }: { eye: 0 | 1; size: number; voice: VoiceState } = $props();

  let canvas: HTMLCanvasElement;

  $effect(() => {
    eyes.show(voice);
  });

  $effect(() => {
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(size * dpr);
    canvas.height = Math.round(size * dpr);
    let frame = requestAnimationFrame(function draw(now) {
      if (eye === 0) {
        // The board keeps its audio level to itself: something voice-like instead.
        const t = now / 1000;
        const level =
          voice === "speaking" ? 0.3 + 0.5 * Math.abs(Math.sin(t * 7.3) * Math.sin(t * 2.1)) : voice === "listening" ? 0.15 * Math.abs(Math.sin(t * 3)) : 0;
        eyes.setLevel(level);
      }
      eyes.step(now);
      drawEye(ctx, eye, size * dpr);
      frame = requestAnimationFrame(draw);
    });
    return () => cancelAnimationFrame(frame);
  });
</script>

<canvas bind:this={canvas} class="eyes" style:width="{size}px" style:height="{size}px" aria-hidden="true"></canvas>

<style>
  .eyes {
    position: absolute;
    inset: 0;
    margin: auto;
    border-radius: 50%;
    pointer-events: none;
  }
</style>
