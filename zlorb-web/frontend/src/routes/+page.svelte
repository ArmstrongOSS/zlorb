<script lang="ts">
    import type { PageProps } from './$types';
	let { data }: PageProps = $props();

    console.log();
</script>

<main>
    <h1 style="text-align: center;">Zlorb Web UI</h1>

    <br />
    <h2>Tracked Repositories</h2>
    <ul>
        {#each data.repositories as repository}
            <li>
                <span>{repository.name}</span>
                <span>({repository.remote}/{repository.branch})</span>
            </li>
        {/each}
    </ul>
</main>

<style>
    main {
        padding: 2.5rem;
    }
</style>
