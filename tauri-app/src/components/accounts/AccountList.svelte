<script lang="ts">
  import type { AccountState } from "../../lib/types";
  import AccountCard from "./AccountCard.svelte";

  interface Props {
    accounts: AccountState[];
  }

  let { accounts: accountList }: Props = $props();

  let sorted = $derived(
    [...accountList].sort((a, b) => {
      // Active first
      if (a.isActive !== b.isActive) return a.isActive ? -1 : 1;
      // Then by priority (lower = higher priority)
      const pa = a.data.priority ?? 99;
      const pb = b.data.priority ?? 99;
      return pa - pb;
    })
  );
</script>

<div class="account-grid">
  {#each sorted as account (account.key)}
    <AccountCard {account} />
  {/each}

  {#if sorted.length === 0}
    <div class="empty-state">
      <p class="empty-title">Aucun compte configure</p>
      <p class="empty-desc">Ajoutez un compte pour commencer</p>
    </div>
  {/if}
</div>

<style>
  .account-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: 12px;
  }

  .empty-state {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 48px;
    text-align: center;
  }

  .empty-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--fg-secondary);
    margin-bottom: 4px;
  }

  .empty-desc {
    font-size: 13px;
    color: var(--fg-dim);
  }
</style>
