<script lang="ts">
  import type { Provider, ModelTier } from "../../lib/types";
  import Button from "../ui/Button.svelte";
  import { Save } from "lucide-svelte";

  interface Props {
    mappings?: Record<Provider, ModelTier>;
    onsave?: (mappings: Record<Provider, ModelTier>) => void;
  }

  const defaultMappings: Record<Provider, ModelTier> = {
    anthropic: { opus: "claude-opus-4-20250514", sonnet: "claude-sonnet-4-20250514", haiku: "claude-haiku-4-20250514" },
    gemini: { opus: "gemini-2.5-pro", sonnet: "gemini-2.5-flash", haiku: "gemini-2.0-flash-lite" },
    openai: { opus: "o3", sonnet: "gpt-4.1", haiku: "gpt-4.1-mini" },
    xai: { opus: "grok-3", sonnet: "grok-3-mini", haiku: "grok-2" },
    deepseek: { opus: "deepseek-r1", sonnet: "deepseek-chat", haiku: "deepseek-chat" },
    mistral: { opus: "mistral-large-latest", sonnet: "mistral-medium-latest", haiku: "mistral-small-latest" },
    groq: { opus: "llama-3.3-70b-versatile", sonnet: "llama-3.1-8b-instant", haiku: "gemma2-9b-it" },
  };

  let { mappings = defaultMappings, onsave }: Props = $props();

  let editMappings: Record<Provider, ModelTier> = $state(JSON.parse(JSON.stringify(defaultMappings)));

  $effect(() => {
    editMappings = JSON.parse(JSON.stringify(mappings));
  });

  const providers: Provider[] = ["anthropic", "gemini", "openai", "xai", "deepseek", "mistral", "groq"];
  const tiers: (keyof ModelTier)[] = ["opus", "sonnet", "haiku"];

  const providerColors: Record<Provider, string> = {
    anthropic: "var(--provider-anthropic)",
    gemini: "var(--provider-gemini)",
    openai: "var(--provider-openai)",
    xai: "var(--provider-xai)",
    deepseek: "var(--provider-deepseek)",
    mistral: "var(--provider-mistral)",
    groq: "var(--provider-groq)",
  };

  function handleSave() {
    onsave?.(editMappings);
  }
</script>

<div class="model-mapping">
  <div class="mapping-header">
    <h3 class="mapping-title">Mapping des modeles</h3>
    <Button variant="primary" size="sm" onclick={handleSave}>
      <Save size={14} />
      Sauvegarder
    </Button>
  </div>

  <div class="mapping-table-wrapper">
    <table class="mapping-table">
      <thead>
        <tr>
          <th class="th-provider">Provider</th>
          {#each tiers as tier}
            <th class="th-tier">{tier.charAt(0).toUpperCase() + tier.slice(1)}</th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#each providers as provider}
          <tr>
            <td class="td-provider">
              <span class="provider-dot" style="background: {providerColors[provider]}"></span>
              <span class="provider-name">{provider}</span>
            </td>
            {#each tiers as tier}
              <td class="td-model">
                <input
                  type="text"
                  class="model-input"
                  bind:value={editMappings[provider][tier]}
                />
              </td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

<style>
  .model-mapping {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .mapping-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .mapping-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .mapping-table-wrapper {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  .mapping-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  .mapping-table th {
    padding: 10px 12px;
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    background: var(--bg-app);
    border-bottom: 1px solid var(--border);
  }

  .mapping-table td {
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
  }

  .mapping-table tr:last-child td {
    border-bottom: none;
  }

  .td-provider {
    display: flex;
    align-items: center;
    gap: 8px;
    white-space: nowrap;
  }

  .provider-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .provider-name {
    font-weight: 500;
    color: var(--fg-primary);
    text-transform: capitalize;
  }

  .td-model {
    min-width: 180px;
  }

  .model-input {
    width: 100%;
    padding: 4px 8px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-primary);
    font-size: 12px;
    font-family: "JetBrains Mono", "Fira Code", monospace;
  }

  .model-input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-glow);
  }
</style>
