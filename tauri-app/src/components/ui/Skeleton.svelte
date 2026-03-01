<script lang="ts">
  interface Props {
    width?: string;
    height?: string;
    rounded?: boolean;
    circle?: boolean;
  }

  let { width = "100%", height = "16px", rounded = true, circle = false }: Props = $props();
</script>

<div
  class="skeleton"
  class:rounded
  class:circle
  style="width: {circle ? height : width}; height: {height}"
></div>

<style>
  .skeleton {
    background: linear-gradient(90deg, var(--bg-card) 25%, var(--bg-card-hover) 50%, var(--bg-card) 75%);
    background-size: 200% 100%;
    animation: shimmer 1.5s ease-in-out infinite;
  }

  .rounded { border-radius: var(--radius-md); }
  .circle { border-radius: 50%; }

  @keyframes shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }
</style>
