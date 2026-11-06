import { Children, useLayoutEffect, useRef, type ReactNode } from 'react';

export function Masonry({ children }: { children: ReactNode }) {
  const container = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const items = Array.from(container.current?.children || []) as HTMLElement[];
    const layout = () => {
      const heights = items.map(item => Math.ceil(item.getBoundingClientRect().height));
      items.forEach((item, index) => {
        item.style.gridRowEnd = `span ${Math.max(1, heights[index])}`;
      });
    };

    // Measure natural card heights so expanding repositories and resizing both reflow the grid.
    layout();
    const observer = new ResizeObserver(layout);
    items.forEach(item => observer.observe(item));
    return () => observer.disconnect();
  }, [children]);

  return <div className="cards" ref={container}>
    {Children.map(children, child => <div className="masonry-item">{child}</div>)}
  </div>;
}
