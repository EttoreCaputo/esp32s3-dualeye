<!-- An on/off setting: a short label, the longer story behind an ⓘ. -->
<script lang="ts">
  import InfoTip from "../InfoTip.svelte";

  let {
    checked,
    label,
    tip,
    title,
    disabled = false,
    master = false,
    anchor,
    onchange,
  }: {
    checked: boolean;
    label: string;
    tip?: string;
    /** Why it's disabled, on hover. */
    title?: string;
    disabled?: boolean;
    master?: boolean;
    anchor?: string;
    onchange: (on: boolean) => void;
  } = $props();
</script>

<label class="switch" class:master title={title || undefined} data-anchor={anchor}>
  <input type="checkbox" {checked} {disabled} onchange={(e) => onchange(e.currentTarget.checked)} />
  <span class="track"><span class="knob"></span></span>
  <span class="slabel">{label}{#if tip}&nbsp;<InfoTip text={tip} />{/if}</span>
</label>
