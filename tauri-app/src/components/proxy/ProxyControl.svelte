<script lang="ts">
  import { proxyInstances } from "../../lib/stores/proxy";
  import { detectProxyBinaries } from "../../lib/tauri";
  import ProxyInstanceCard from "./ProxyInstanceCard.svelte";
  import Card from "../ui/Card.svelte";
  import Button from "../ui/Button.svelte";
  import { Plus } from "lucide-svelte";
  import { onMount } from "svelte";
  import type { ProxyInstanceState, ProxyInstanceConfig, ProxyKind } from "../../lib/types";

  interface DetectedBinary {
    id: string;
    name: string;
    path: string;
    defaultPort: number;
  }

  let instances: ProxyInstanceState[] = $state([]);
  let detectedBinaries: DetectedBinary[] = $state([]);
  let showAdd = $state(false);
  let newName = $state("");
  let newPort = $state(8082);
  let newKind: ProxyKind = $state("router");
  let newBinaryPath = $state("");

  onMount(() => {
    // Probe détecte aussi les proxys externes (V2, P2P)
    proxyInstances.probe();
    detectProxyBinaries().then((bins) => { detectedBinaries = bins; });
    const unsub = proxyInstances.subscribe((list) => {
      instances = list;
    });
    return unsub;
  });

  function openAdd() {
    newName = "";
    newPort = 8082;
    newKind = "router";
    newBinaryPath = "";
    showAdd = true;
  }

  function onBinaryChange(e: Event) {
    const val = (e.target as HTMLSelectElement).value;
    newBinaryPath = val;
    if (val) {
      const bin = detectedBinaries.find((b) => b.path === val);
      if (bin) {
        newPort = bin.defaultPort;
        if (!newName.trim()) newName = bin.name;
        if (bin.id.includes("router")) newKind = "router";
        else if (bin.id.includes("impersonator")) newKind = "impersonator";
        else newKind = "custom";
      }
    }
  }

  async function addInstance() {
    if (!newName.trim()) return;
    const id = newName.toLowerCase().replace(/[^a-z0-9]/g, "-") + "-" + Date.now().toString(36);
    const config: ProxyInstanceConfig = {
      id,
      name: newName.trim(),
      kind: newKind,
      port: newPort,
      autoStart: false,
      enabled: true,
      binaryPath: newBinaryPath || undefined,
      setupTargets: [],
    };
    await proxyInstances.add(config);
    showAdd = false;
  }

  function cancelAdd() {
    showAdd = false;
  }

  export { detectedBinaries };
</script>

<div class="proxy-control">
  <div class="instances-grid">
    {#each instances as instance (instance.config.id)}
      <ProxyInstanceCard {instance} {detectedBinaries} />
    {/each}
  </div>

  {#if showAdd}
    <Card>
      <div class="add-form">
        <h3 class="add-title">Nouveau proxy</h3>
        <div class="add-fields-top">
          <div class="field field-grow">
            <label class="field-label" for="proxy-binary">Moteur</label>
            <select id="proxy-binary" class="field-input" value={newBinaryPath} onchange={onBinaryChange}>
              <option value="">Integre (V3)</option>
              {#each detectedBinaries as bin}
                <option value={bin.path}>{bin.name}</option>
              {/each}
            </select>
          </div>
        </div>
        <div class="add-fields">
          <div class="field">
            <label class="field-label" for="proxy-name">Nom</label>
            <input
              id="proxy-name"
              class="field-input"
              type="text"
              bind:value={newName}
              placeholder="Mon Proxy"
            />
          </div>
          <div class="field">
            <label class="field-label" for="proxy-port">Port</label>
            <input
              id="proxy-port"
              class="field-input port-input"
              type="number"
              bind:value={newPort}
              min="1024"
              max="65535"
            />
          </div>
          <div class="field">
            <label class="field-label" for="proxy-kind">Type</label>
            <select id="proxy-kind" class="field-input" bind:value={newKind}>
              <option value="router">Router</option>
              <option value="impersonator">Anthrouter</option>
              <option value="custom">Custom</option>
            </select>
          </div>
        </div>
        <div class="add-actions">
          <Button variant="primary" size="sm" onclick={addInstance}>Ajouter</Button>
          <Button variant="ghost" size="sm" onclick={cancelAdd}>Annuler</Button>
        </div>
      </div>
    </Card>
  {:else}
    <button class="add-button" onclick={openAdd}>
      <Plus size={16} />
      Ajouter un proxy
    </button>
  {/if}
</div>

<style>
  .proxy-control {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .instances-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 16px;
  }

  .add-button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 12px;
    border: 2px dashed var(--border);
    border-radius: var(--radius-lg);
    background: none;
    color: var(--fg-dim);
    font-size: 13px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .add-button:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--bg-card-hover);
  }

  .add-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .add-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .add-fields-top {
    display: flex;
    gap: 10px;
  }

  .add-fields {
    display: grid;
    grid-template-columns: 1fr 100px 130px;
    gap: 10px;
    align-items: end;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field-grow {
    flex: 1;
  }

  .field-label {
    font-size: 11px;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .field-input {
    padding: 6px 10px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-primary);
    font-size: 13px;
  }

  .field-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .port-input {
    text-align: center;
  }

  .add-actions {
    display: flex;
    gap: 8px;
  }
</style>
