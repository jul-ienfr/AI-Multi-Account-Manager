<script lang="ts">
  import type { RoutingStrategy } from "../../lib/types";
  import Card from "../ui/Card.svelte";
  import { Target, BarChart3, RefreshCw, Clock, TrendingUp, GripVertical } from "lucide-svelte";

  interface Props {
    selected?: RoutingStrategy;
    onchange?: (strategy: RoutingStrategy) => void;
    onreorder?: (order: RoutingStrategy[]) => void;
  }

  let { selected = $bindable("priority"), onchange, onreorder }: Props = $props();

  let strategies: Array<{
    id: RoutingStrategy;
    name: string;
    description: string;
    icon: typeof Target;
  }> = $state([
    {
      id: "priority",
      name: "Priorite",
      description: "Utilise le compte avec la priorite la plus haute. Bascule uniquement quand le compte actif atteint ses limites.",
      icon: Target,
    },
    {
      id: "quota-aware",
      name: "Quota-Aware",
      description: "Choisit automatiquement le compte avec le plus de quota disponible. Equilibre la charge intelligemment.",
      icon: BarChart3,
    },
    {
      id: "round-robin",
      name: "Round Robin",
      description: "Alterne entre les comptes de facon cyclique. Repartition equitable des requetes.",
      icon: RefreshCw,
    },
    {
      id: "latency",
      name: "Latence",
      description: "Selectionne le compte avec la meilleure latence mesuree. Optimise la reactivite.",
      icon: Clock,
    },
    {
      id: "usage-based",
      name: "Usage-Based",
      description: "Repartit selon l'utilisation cumulee. Equilibre le cout entre les comptes.",
      icon: TrendingUp,
    },
  ]);

  let dragIdx: number | null = $state(null);
  let dragOverIdx: number | null = $state(null);

  function selectStrategy(id: RoutingStrategy) {
    selected = id;
    onchange?.(id);
  }

  function handleDragStart(idx: number) {
    dragIdx = idx;
  }

  function handleDragOver(e: DragEvent, idx: number) {
    e.preventDefault();
    dragOverIdx = idx;
  }

  function handleDrop(idx: number) {
    if (dragIdx !== null && dragIdx !== idx) {
      const reordered = [...strategies];
      const [moved] = reordered.splice(dragIdx, 1);
      reordered.splice(idx, 0, moved);
      strategies = reordered;
      onreorder?.(reordered.map(s => s.id));
    }
    dragIdx = null;
    dragOverIdx = null;
  }

  function handleDragEnd() {
    dragIdx = null;
    dragOverIdx = null;
  }
</script>

<div class="strategy-grid">
  {#each strategies as strategy, i}
    <div
      class="strategy-drag-wrapper"
      class:dragging={dragIdx === i}
      class:drag-over={dragOverIdx === i && dragIdx !== i}
      draggable="true"
      role="listitem"
      ondragstart={() => handleDragStart(i)}
      ondragover={(e) => handleDragOver(e, i)}
      ondrop={() => handleDrop(i)}
      ondragend={handleDragEnd}
    >
      <Card
        active={selected === strategy.id}
        onclick={() => selectStrategy(strategy.id)}
      >
        <div class="strategy-card">
          <div class="strategy-header">
            <span class="strategy-icon" class:active={selected === strategy.id}>
              <strategy.icon size={20} />
            </span>
            <div class="strategy-right">
              <span class="drag-handle" aria-label="Glisser pour reordonner">
                <GripVertical size={14} />
              </span>
              <div class="strategy-radio" class:selected={selected === strategy.id}>
                {#if selected === strategy.id}
                  <span class="radio-dot"></span>
                {/if}
              </div>
            </div>
          </div>
          <h4 class="strategy-name">{strategy.name}</h4>
          <p class="strategy-desc">{strategy.description}</p>
        </div>
      </Card>
    </div>
  {/each}
</div>

<style>
  .strategy-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 12px;
  }

  .strategy-drag-wrapper {
    transition: transform 0.15s ease, opacity 0.15s ease;
    cursor: grab;
  }

  .strategy-drag-wrapper:active { cursor: grabbing; }

  .strategy-drag-wrapper.dragging {
    opacity: 0.4;
    transform: scale(0.96);
  }

  .strategy-drag-wrapper.drag-over {
    border-left: 2px solid var(--accent);
    border-radius: var(--radius-md);
  }

  .strategy-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .strategy-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .strategy-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .drag-handle {
    display: flex;
    color: var(--fg-dim);
    opacity: 0.4;
    cursor: grab;
    transition: opacity 0.15s ease;
  }

  .strategy-drag-wrapper:hover .drag-handle { opacity: 1; }

  .strategy-icon {
    display: flex;
    color: var(--fg-dim);
    transition: color 0.2s ease;
  }

  .strategy-icon.active {
    color: var(--accent);
  }

  .strategy-radio {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: border-color 0.2s ease;
  }

  .strategy-radio.selected {
    border-color: var(--accent);
  }

  .radio-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
  }

  .strategy-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .strategy-desc {
    font-size: 12px;
    color: var(--fg-secondary);
    line-height: 1.5;
  }
</style>
