export function Spinner({ size = 3 }: { size?: number }) {
  return (
    <div className={`w-${size} h-${size} border-2 border-white/20 border-t-white rounded-full animate-spin`} />
  );
}
