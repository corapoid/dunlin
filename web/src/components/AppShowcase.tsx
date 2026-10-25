import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { CircleNotchIcon } from "@phosphor-icons/react";

export interface ShowcaseShot {
  src: string;
  alt: string;
}

interface Props {
  shots: {
    chat: ShowcaseShot;
    graph: ShowcaseShot;
    trace: ShowcaseShot;
  };
}

const tabs = ["chat", "graph", "trace"] as const;
type Tab = (typeof tabs)[number];

const labels: Record<Tab, string> = {
  chat: "Chat",
  graph: "Graph",
  trace: "Trace",
};

export default function AppShowcase({ shots }: Props) {
  const [active, setActive] = useState<Tab>("chat");
  const [paused, setPaused] = useState(false);
  const [inView, setInView] = useState(true);
  const rootRef = useRef<HTMLDivElement>(null);
  const reduceMotion = useReducedMotion();

  const activeIndex = tabs.indexOf(active);
  const back = tabs[(activeIndex + 1) % tabs.length];

  const ordered = useMemo(
    () => tabs.map((tab) => ({ tab, ...shots[tab] })),
    [shots],
  );

  useEffect(() => {
    const element = rootRef.current;
    if (!element) {
      return;
    }

    const observer = new IntersectionObserver(
      ([entry]) => setInView(entry.isIntersecting),
      { threshold: 0.15 },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (paused || reduceMotion || !inView) {
      return;
    }

    const timer = window.setInterval(() => {
      setActive((current) => {
        const index = tabs.indexOf(current);
        return tabs[(index + 1) % tabs.length];
      });
    }, 5200);

    return () => window.clearInterval(timer);
  }, [paused, reduceMotion, active, inView]);

  return (
    <div
      ref={rootRef}
      className="relative min-w-0"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
      onFocusCapture={() => setPaused(true)}
      onBlurCapture={() => setPaused(false)}
    >
      <div className="relative mx-auto w-full max-w-[760px] lg:max-w-none">
        <div
          aria-hidden="true"
          className="absolute inset-x-8 bottom-0 top-16 rounded-2xl border border-ink/10 bg-paper-deep/70"
        />

        <motion.div
          aria-hidden="true"
          className="absolute -right-3 -top-7 hidden w-[88%] overflow-hidden rounded-2xl border border-ink/10 shadow-[0_30px_80px_rgba(18,17,14,0.12)] sm:block"
          initial={reduceMotion ? false : { opacity: 0, y: -24, rotate: 4 }}
          animate={{ opacity: 0.72, y: 0, rotate: 2.2 }}
          transition={{ duration: 1.2, delay: 0.18, ease: [0.16, 1, 0.3, 1] }}
        >
          <img
            src={shots[back].src}
            alt=""
            width="1600"
            height="1012"
            className="block h-auto w-full"
            loading="eager"
            decoding="async"
          />
        </motion.div>

        <motion.div
          className="group relative overflow-hidden rounded-2xl border border-night-line bg-night shadow-[0_40px_100px_rgba(18,17,14,0.24)] transition-transform duration-700 ease-fluid hover:scale-[1.008]"
          initial={
            reduceMotion
              ? false
              : { opacity: 0, y: 48, rotate: -2, filter: "blur(14px)" }
          }
          animate={{ opacity: 1, y: 0, rotate: -1, filter: "blur(0px)" }}
          transition={{ duration: 1.15, ease: [0.16, 1, 0.3, 1] }}
        >
          <div className="relative aspect-[1800/1138] w-full">
            <AnimatePresence initial={false}>
              <motion.img
                key={active}
                src={shots[active].src}
                alt={shots[active].alt}
                width="1600"
                height="1012"
                className="absolute inset-0 size-full object-cover"
                initial={reduceMotion ? false : { opacity: 0, scale: 1.015 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={reduceMotion ? undefined : { opacity: 0 }}
                transition={{ duration: 0.7, ease: [0.32, 0.72, 0, 1] }}
                loading="eager"
                decoding="async"
              />
            </AnimatePresence>
          </div>

          <div className="pointer-events-none absolute left-4 top-4 hidden items-center gap-2 rounded-full border border-white/10 bg-black/45 px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.12em] text-white/75 backdrop-blur-sm sm:flex">
            <span className="pulse-dot size-1.5 rounded-full bg-[#4f9b74]" />
            ACP · live
          </div>
        </motion.div>

        <motion.div
          className="absolute -bottom-5 -left-3 hidden sm:block"
          initial={reduceMotion ? false : { opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.8, delay: 0.55, ease: [0.16, 1, 0.3, 1] }}
        >
          <div className="float-slow flex items-center gap-2.5 rounded-full border border-ink/10 bg-paper-soft px-3.5 py-2 shadow-[0_12px_30px_rgba(18,17,14,0.1)]">
            <CircleNotchIcon size={14} className="text-moss" />
            <span className="font-mono text-[11px] uppercase tracking-[0.1em] text-ink/65">
              1 turn · 3 tools
            </span>
          </div>
        </motion.div>
      </div>

      <div className="mt-7 flex items-center justify-center gap-1 rounded-full border border-ink/10 bg-paper-soft/70 p-1 sm:mt-8 sm:w-max sm:mx-auto">
        <div
          role="tablist"
          aria-label="Product views"
          className="flex items-center gap-1"
        >
          {ordered.map(({ tab }) => {
            const selected = tab === active;
            return (
              <button
                key={tab}
                role="tab"
                type="button"
                aria-selected={selected}
                aria-label={`Show ${labels[tab]} view`}
                onClick={() => setActive(tab)}
                className={`relative rounded-full px-4 py-2 text-sm font-medium transition-colors duration-500 ease-fluid ${
                  selected ? "text-paper-soft" : "text-ink/55 hover:text-ink"
                }`}
              >
                {selected && (
                  <motion.span
                    layoutId="showcase-tab"
                    className="absolute inset-0 rounded-full bg-ink"
                    transition={
                      reduceMotion
                        ? { duration: 0 }
                        : { duration: 0.5, ease: [0.32, 0.72, 0, 1] }
                    }
                  />
                )}
                <span className="relative z-10">{labels[tab]}</span>
              </button>
            );
          })}
        </div>
      </div>
      <p className="mt-3 text-center font-mono text-[11px] uppercase tracking-[0.12em] text-muted">
        Real app screenshots · cycles automatically
      </p>
    </div>
  );
}
