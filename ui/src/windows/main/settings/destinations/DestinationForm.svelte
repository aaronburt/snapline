<script lang="ts">
  import type {
    DestinationInput,
    DestinationTestOutcome,
    DestinationView,
  } from '../../../../lib/ipc/upload';
  import CustomForm from './CustomForm.svelte';
  import ImgurForm from './ImgurForm.svelte';
  import S3Form from './S3Form.svelte';

  interface Props {
    view: DestinationView;
    onsave: (input: DestinationInput) => Promise<DestinationView>;
    ontest: (id: string) => Promise<DestinationTestOutcome>;
    onremove: (id: string) => Promise<void>;
    onexport: (id: string, includeHeaderValues: boolean) => Promise<string | null>;
  }

  let { view, onsave, ontest, onremove, onexport }: Props = $props();
</script>

{#if view.kind === 'imgur'}
  <ImgurForm {view} {onsave} {ontest} {onremove} />
{:else if view.kind === 's3'}
  <S3Form {view} {onsave} {ontest} {onremove} />
{:else}
  <CustomForm {view} {onsave} {ontest} {onremove} {onexport} />
{/if}
