import { facts } from "../facts";

export default function OpenSource() {
  return (
    <section className="section container" aria-labelledby="os-title">
      <h2 id="os-title">Open source.</h2>
      <p className="lead">
        MIT OR Apache-2.0. Built on iced, russh, suppaftp and alacritty_terminal.
      </p>
      <p>
        <a href={facts.repoUrl}>Read the code on GitHub</a>
      </p>
    </section>
  );
}
