import type { FilterDescriptor, OverviewUrlState, QuerySummaryChip, ResourceOverview } from '../types';

export function findSelectedResource(
  resources: ResourceOverview[],
  selectedResourceName: string | null
) {
  return resources.find((resource) => resource.name === selectedResourceName) ?? resources[0] ?? null;
}

export function filterResourcesBySearch(resources: ResourceOverview[], search: string) {
  const needle = search.trim().toLowerCase();
  if (!needle) {
    return resources;
  }

  return resources.filter(
    (resource) =>
      resource.name.toLowerCase().includes(needle) ||
      resource.field_names.some((field) => field.toLowerCase().includes(needle))
  );
}

export function removeQuerySummaryChip(
  state: OverviewUrlState,
  chip: QuerySummaryChip
): OverviewUrlState {
  if (chip.kind === 'filter') {
    return {
      ...state,
      page: 1,
      filters: state.filters.filter((filter) => filter.id !== chip.id)
    };
  }

  if (chip.kind === 'sort') {
    const columnId = chip.id.replace('sort:', '');
    return {
      ...state,
      page: 1,
      sorting: state.sorting.filter((sort) => sort.id !== columnId)
    };
  }

  const embed = chip.id.replace('embed:', '');
  return {
    ...state,
    page: 1,
    embeds: state.embeds.filter((entry) => entry !== embed)
  };
}

export function buildDrilldownFilter(field: string, value: string | null): FilterDescriptor {
  return {
    id: `${field}:eq:drilldown`,
    field,
    operator: 'eq',
    value: value ?? ''
  };
}
