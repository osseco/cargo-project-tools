import { ArrowRight, ExternalLink } from "lucide-react";
import { Link } from "react-router-dom";
import { CodeBlock } from "../components/code-block";
import { Eyebrow, Section } from "../components/doc-page";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { GITHUB_REPO } from "../lib/links";

const FEATURES = [
  {
    title: "Your own locks",
    body: "Each tool gets a Cargo.lock in your repository. Changes to it go through review like any other code.",
  },
  {
    title: "Audit and fix",
    body: "cargo-audit checks every lock. When a fix exists, you get the update command and don't wait for a tool release.",
  },
  {
    title: "Built into the project",
    body: "Tools install into .project-tools/bin with --locked, and rebuild only when a version, lock or setting changes.",
  },
];

const QUICK_START = `
cargo project-tools init
cargo project-tools add cargo-nextest
cargo project-tools audit
cargo project-tools install
`;

const LAYOUT = `
<project root>
├── project-tools.toml        # which tools, which versions, build settings
└── .project-tools
    ├── crates
    │   └── <tool>
    │       ├── Cargo.lock          # your lock for this tool
    │       └── audit-ignore.toml   # accepted advisories, with reasons
    ├── bin                   # installed binaries
    └── targets               # build directories, one per tool
`;

/** Inline code outside prose, styled like prose-osseco's. */
function Code({ children }: { children: string }) {
  return (
    <code className="rounded bg-muted px-1.5 py-0.5 text-[0.9em] text-foreground">
      {children}
    </code>
  );
}

export function HomePage() {
  return (
    <>
      <section className="max-w-3xl space-y-5">
        <Eyebrow>A cargo subcommand</Eyebrow>
        <h1 className="text-4xl font-semibold tracking-tight text-balance md:text-5xl">
          Install your project's cargo tools from your own Cargo.lock files.
        </h1>
        <p className="text-lg leading-8 text-pretty text-muted-foreground">
          <Code>cargo install --locked</Code> builds a tool with the lock its
          maintainer published. When a dependency gets a security fix, you wait
          for a new release. <Code>cargo project-tools</Code> keeps a lock per
          tool in your repository, so you can audit it, bump vulnerable
          dependencies yourself, and install the tools into the project.
        </p>
        <div className="flex flex-wrap gap-3">
          <Button asChild>
            <Link to="/docs/getting-started">
              Get started <ArrowRight />
            </Link>
          </Button>
          <Button asChild variant="outline">
            <a href={GITHUB_REPO}>
              GitHub <ExternalLink />
            </a>
          </Button>
        </div>
      </section>

      <div className="mt-12 grid gap-4 md:grid-cols-3">
        {FEATURES.map((feature) => (
          <Card key={feature.title}>
            <CardHeader>
              <CardTitle className="text-base">{feature.title}</CardTitle>
            </CardHeader>
            <CardContent className="text-sm leading-6 text-muted-foreground">
              {feature.body}
            </CardContent>
          </Card>
        ))}
      </div>

      <Section id="quick-start" title="Quick start" className="mt-12 max-w-3xl">
        <p>
          In your project's root, with{" "}
          <a href="https://crates.io/crates/cargo-audit">cargo-audit</a> on your{" "}
          <code>PATH</code>:
        </p>
        <CodeBlock>{QUICK_START}</CodeBlock>
        <div className="not-prose">
          <Button asChild variant="outline" size="sm">
            <Link to="/docs/getting-started">
              Install cargo-project-tools <ArrowRight />
            </Link>
          </Button>
        </div>
      </Section>

      <Section id="layout" title="What lives where" className="mt-12 max-w-3xl">
        <p>
          Commit <code>project-tools.toml</code> and{" "}
          <code>.project-tools/crates/</code>. Everything else is git-ignored.
        </p>
        <CodeBlock copy={false}>{LAYOUT}</CodeBlock>
        <div className="not-prose">
          <Button asChild variant="outline" size="sm">
            <Link to="/docs/how-it-works">
              How it works <ArrowRight />
            </Link>
          </Button>
        </div>
      </Section>
    </>
  );
}
