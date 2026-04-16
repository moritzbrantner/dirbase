import { buildBrowserQueryString, buildResourceSearchParams, parseOverviewState } from './urlState';

describe('overview URL state', () => {
  it('hydrates state from the browser query string', () => {
    const state = parseOverviewState(
      '?resource=posts&view=raw&page=2&per_page=50&sort=-title,author&embed=author_id&title:contains=hello&status=draft'
    );

    expect(state.resource).toBe('posts');
    expect(state.view).toBe('raw');
    expect(state.page).toBe(2);
    expect(state.perPage).toBe(50);
    expect(state.sorting).toEqual([
      { id: 'title', desc: true },
      { id: 'author', desc: false }
    ]);
    expect(state.embeds).toEqual(['author_id']);
    expect(state.filters).toEqual([
      expect.objectContaining({ field: 'title', operator: 'contains', value: 'hello' }),
      expect.objectContaining({ field: 'status', operator: 'eq', value: 'draft' })
    ]);
  });

  it('serializes filters into REST query params', () => {
    const params = buildResourceSearchParams({
      page: 1,
      perPage: 25,
      sorting: [],
      embeds: [],
      filters: [
        { id: '1', field: 'title', operator: 'contains', value: 'hello' },
        { id: '2', field: 'status', operator: 'eq', value: 'draft' }
      ]
    });

    expect(params.toString()).toBe('page=1&per_page=25&title%3Acontains=hello&status=draft');
  });

  it('serializes sorting and pagination into the browser query string', () => {
    const queryString = buildBrowserQueryString({
      resource: 'users',
      view: 'explore',
      page: 3,
      perPage: 100,
      sorting: [
        { id: 'last_name', desc: false },
        { id: 'age', desc: true }
      ],
      filters: [],
      embeds: ['manager_id']
    });

    expect(queryString).toBe('?resource=users&page=3&per_page=100&sort=last_name%2C-age&embed=manager_id');
  });

  it('serializes null operators without a value payload', () => {
    const params = buildResourceSearchParams({
      page: 1,
      perPage: 25,
      sorting: [],
      embeds: [],
      filters: [
        { id: '1', field: 'deleted_at', operator: 'isNull', value: '' },
        { id: '2', field: 'published_at', operator: 'isNotNull', value: '' }
      ]
    });

    expect(params.toString()).toBe(
      'page=1&per_page=25&deleted_at%3AisNull=true&published_at%3AisNotNull=true'
    );
  });
});
