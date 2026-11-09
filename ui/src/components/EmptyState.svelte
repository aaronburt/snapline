<script lang="ts">
  interface Props {
    title: string;
    text?: string;
  }

  let { title, text }: Props = $props();
</script>

<div class="empty">
  <p class="title">{title}</p>
  {#if text}
    <p class="text">{text}</p>
  {/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    text-align: center;
  }

  p {
    margin: 0;
  }

  .title {
    font-size: 14px;
    font-weight: 600;
  }

  .text {
    font-size: 12px;
    color: var(--text3);
  }
</style>
