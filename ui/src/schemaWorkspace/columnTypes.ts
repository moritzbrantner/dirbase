export const COLUMN_TYPES = new Set([
  'integer',
  'float',
  'boolean',
  'string',
  'json',
  'date',
  'datetime',
  'uuid',
  'big_integer',
  'decimal'
]);

export function columnTypesAreCompatible(left: string, right: string): boolean {
  return (
    left === right ||
    (left === 'integer' && right === 'float') ||
    (left === 'float' && right === 'integer') ||
    (isNumericLikeColumnType(left) && isNumericLikeColumnType(right))
  );
}

export function isKnownComparableColumnType(columnType: string): boolean {
  return COLUMN_TYPES.has(columnType);
}

export function isNumericLikeColumnType(columnType: string): boolean {
  return ['integer', 'float', 'big_integer', 'decimal'].includes(columnType);
}

export function isBoundedColumnType(columnType: string): boolean {
  return isNumericLikeColumnType(columnType) || ['date', 'datetime'].includes(columnType);
}

export function isStringBackedColumnType(columnType: string): boolean {
  return ['string', 'date', 'datetime', 'uuid', 'big_integer', 'decimal'].includes(columnType);
}
