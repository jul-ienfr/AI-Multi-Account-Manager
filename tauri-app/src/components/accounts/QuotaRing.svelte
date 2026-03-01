<script lang="ts">
  import type { QuotaPhase } from "../../lib/types";

  interface Props {
    percent: number;
    phase?: QuotaPhase;
    size?: number;
    strokeWidth?: number;
  }

  let { percent = 0, phase = "Cruise", size = 56, strokeWidth = 4 }: Props = $props();

  const phaseColors: Record<QuotaPhase, string> = {
    Cruise: "var(--phase-cruise)",
    Watch: "var(--phase-watch)",
    Alert: "var(--phase-alert)",
    Critical: "var(--phase-critical)",
  };

  let radius = $derived((size - strokeWidth) / 2);
  let circumference = $derived(2 * Math.PI * radius);
  let offset = $derived(circumference - Math.min(percent, 1) * circumference);
  let color = $derived(phaseColors[phase ?? "Cruise"]);
  let displayPercent = $derived(Math.round(Math.min(percent, 1) * 100));
</script>

<div class="quota-ring" style="width: {size}px; height: {size}px">
  <svg viewBox="0 0 {size} {size}" class="ring-svg">
    <!-- Background track -->
    <circle
      cx={size / 2}
      cy={size / 2}
      r={radius}
      fill="none"
      stroke="var(--border)"
      stroke-width={strokeWidth}
    />
    <!-- Progress arc -->
    <circle
      cx={size / 2}
      cy={size / 2}
      r={radius}
      fill="none"
      stroke={color}
      stroke-width={strokeWidth}
      stroke-linecap="round"
      stroke-dasharray={circumference}
      stroke-dashoffset={offset}
      class="ring-progress"
      transform="rotate(-90 {size / 2} {size / 2})"
    />
  </svg>
  <span class="ring-label" style="color: {color}; font-size: {size * 0.22}px">
    {displayPercent}%
  </span>
</div>

<style>
  .quota-ring {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .ring-svg {
    width: 100%;
    height: 100%;
  }

  .ring-progress {
    transition: stroke-dashoffset 0.6s ease, stroke 0.3s ease;
    filter: drop-shadow(0 0 4px currentColor);
  }

  .ring-label {
    position: absolute;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
</style>
