<script lang="ts">
  import Card from "../ui/Card.svelte";
  import Badge from "../ui/Badge.svelte";
  import type { Provider } from "../../lib/types";

  const providers: Array<{
    id: Provider;
    name: string;
    color: string;
    description: string;
  }> = [
    { id: "anthropic", name: "Anthropic", color: "var(--provider-anthropic)", description: "Claude via API directe" },
    { id: "gemini", name: "Google Gemini", color: "var(--provider-gemini)", description: "Gemini via Google AI Studio" },
    { id: "openai", name: "OpenAI", color: "var(--provider-openai)", description: "GPT / o-series via API" },
    { id: "xai", name: "xAI", color: "var(--provider-xai)", description: "Grok via API xAI" },
    { id: "deepseek", name: "DeepSeek", color: "var(--provider-deepseek)", description: "DeepSeek R1 / Chat" },
    { id: "mistral", name: "Mistral", color: "var(--provider-mistral)", description: "Mistral AI models" },
    { id: "groq", name: "Groq", color: "var(--provider-groq)", description: "Inference rapide via Groq" },
  ];

  interface ProviderConfig {
    enabled: boolean;
    apiKey: string;
    endpoint: string;
  }

  let configs: Record<Provider, ProviderConfig> = $state({
    anthropic: { enabled: true, apiKey: "", endpoint: "https://api.anthropic.com" },
    gemini: { enabled: false, apiKey: "", endpoint: "https://generativelanguage.googleapis.com" },
    openai: { enabled: false, apiKey: "", endpoint: "https://api.openai.com" },
    xai: { enabled: false, apiKey: "", endpoint: "https://api.x.ai" },
    deepseek: { enabled: false, apiKey: "", endpoint: "https://api.deepseek.com" },
    mistral: { enabled: false, apiKey: "", endpoint: "https://api.mistral.ai" },
    groq: { enabled: false, apiKey: "", endpoint: "https://api.groq.com" },
  });

  let expandedProvider: Provider | null = $state(null);

  function toggleExpand(id: Provider) {
    expandedProvider = expandedProvider === id ? null : id;
  }
</script>

<div class="provider-settings">
  <h3 class="section-title">Providers</h3>
  <p class="section-desc">Configuration des fournisseurs d'API pour le proxy multi-provider.</p>

  <div class="provider-list">
    {#each providers as provider}
      {@const cfg = configs[provider.id]}
      <Card
        onclick={() => toggleExpand(provider.id)}
        active={expandedProvider === provider.id}
      >
        <div class="provider-header">
          <span class="provider-dot" style="background: {provider.color}"></span>
          <div class="provider-info">
            <span class="provider-name">{provider.name}</span>
            <span class="provider-desc">{provider.description}</span>
          </div>
          <Badge color={cfg.enabled ? "var(--status-running)" : "var(--status-stopped)"} small>
            {cfg.enabled ? "Actif" : "Inactif"}
          </Badge>
        </div>

        {#if expandedProvider === provider.id}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="provider-details" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
            <div class="detail-row">
              <label class="detail-label" for="apikey-{provider.id}">Cle API</label>
              <input
                id="apikey-{provider.id}"
                type="password"
                class="detail-input"
                placeholder="sk-..."
                bind:value={configs[provider.id].apiKey}
              />
            </div>
            <div class="detail-row">
              <label class="detail-label" for="endpoint-{provider.id}">Endpoint</label>
              <input
                id="endpoint-{provider.id}"
                type="url"
                class="detail-input"
                bind:value={configs[provider.id].endpoint}
              />
            </div>
          </div>
        {/if}
      </Card>
    {/each}
  </div>
</div>

<style>
  .provider-settings {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .section-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .section-desc {
    font-size: 12px;
    color: var(--fg-dim);
    margin-top: -8px;
  }

  .provider-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .provider-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .provider-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .provider-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .provider-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .provider-desc {
    font-size: 11px;
    color: var(--fg-dim);
  }

  .provider-details {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .detail-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .detail-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--fg-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .detail-input {
    padding: 6px 10px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-primary);
    font-size: 12px;
    font-family: "JetBrains Mono", "Fira Code", monospace;
  }

  .detail-input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-glow);
  }
</style>
