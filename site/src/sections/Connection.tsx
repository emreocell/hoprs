export default function Connection() {
  return (
    <section className="section container" aria-labelledby="conn-title">
      <h2 id="conn-title">One connection, two tabs.</h2>
      <p className="lead">
        A terminal tab and a files tab share one SSH connection. Add more tabs over the same session.
      </p>
      <figure data-slot="shot-split" className="shot" />
    </section>
  );
}
