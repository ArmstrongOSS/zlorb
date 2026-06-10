import type { PageLoad } from './$types';

export async function load({ params, fetch }) {
    const res = await fetch('/api/repositories');
    const data = await res.json();

    return { repositories: data };
}