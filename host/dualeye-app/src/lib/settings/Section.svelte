<!-- A group of settings under a small heading, with its ⓘ. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import InfoTip from "../InfoTip.svelte";

  let {
    title,
    tip,
    anchor,
    class: cls = "",
    dimmed = false,
    children,
  }: { title?: string; tip?: string; anchor?: string; class?: string; dimmed?: boolean; children: Snippet } = $props();
</script>

<section class={cls} class:dimmed data-anchor={anchor}>
  {#if title}<h3>{title}{#if tip}&nbsp;<InfoTip text={tip} />{/if}</h3>{/if}
  {@render children()}
</section>
