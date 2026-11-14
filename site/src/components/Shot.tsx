// A screenshot of the app, captured by site/scripts/capture/run.sh: AVIF and WebP at
// 1440 and 2880 wide (the app ran at scale 2), PNG 1440 as the fallback.
// The shot spans the container: viewport less the gutters, at most 1440 - 2 x 64.
const sizes = "(max-width: 40rem) calc(100vw - 32px), (max-width: 60rem) calc(100vw - 48px), min(calc(100vw - 128px), 1312px)";

export default function Shot({ name, slot, alt, lazy = false }: { name: "main" | "split"; slot: string; alt: string; lazy?: boolean }) {
  const set = (ext: string) => `/shots/${name}-1440.${ext} 1440w, /shots/${name}.${ext} 2880w`;
  return (
    <figure data-slot={slot} className="shot">
      <picture>
        <source type="image/avif" srcSet={set("avif")} sizes={sizes} />
        <source type="image/webp" srcSet={set("webp")} sizes={sizes} />
        <img src={`/shots/${name}.png`} width={1440} height={900} alt={alt} decoding="async" loading={lazy ? "lazy" : undefined} />
      </picture>
    </figure>
  );
}
