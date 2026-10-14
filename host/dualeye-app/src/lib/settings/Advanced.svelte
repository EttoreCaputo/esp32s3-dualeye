<!-- A tab's less used settings: closed at first, then as you left it. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { i18n } from "../i18n/index.svelte";
  import { settingsNav, type Tab } from "./nav.svelte";

  let { tab, children }: { tab: Tab; children: Snippet } = $props();
</script>

<details class="more advanced" open={settingsNav.advanced[tab] ?? false} ontoggle={(e) => settingsNav.setAdvanced(tab, e.currentTarget.open)}>
  <summary>{i18n.t.settings.common.advanced}</summary>
  <div class="advbody">{@render children()}</div>
</details>
