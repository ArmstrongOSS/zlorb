<script lang="ts">
    import '../app.css';
	let { children } = $props();
</script>

<svelte:head>
	<title>Zlorb Web UI</title>
</svelte:head>

{@render children()}
