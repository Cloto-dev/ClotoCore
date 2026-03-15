const sizeClass: Record<number, string> = {
  1: 'w-1 h-1',
  2: 'w-2 h-2',
  3: 'w-3 h-3',
  4: 'w-4 h-4',
  5: 'w-5 h-5',
  6: 'w-6 h-6',
};

export function Spinner({ size = 3 }: { size?: number }) {
  return (
    <div
      className={`${sizeClass[size] || sizeClass[3]} border-2 border-white/20 border-t-white rounded-full animate-spin`}
    />
  );
}
