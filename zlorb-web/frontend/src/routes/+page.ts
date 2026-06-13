import type { PageLoadEvent } from './$types';

export async function load({ params, fetch }: PageLoadEvent) {
    const res = await fetch('/api/repositories');
    return { repositories: await res.json() };
}