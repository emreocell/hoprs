import Shot from "../components/Shot";

export default function Connection() {
  return (
    <section className="section container" aria-labelledby="conn-title">
      <h2 id="conn-title">One connection, two tabs.</h2>
      <p className="lead">
        A terminal tab and a files tab share one SSH connection. Add more tabs over the same session.
      </p>
      <Shot
        name="split"
        slot="shot-split"
        alt="hoprs terminal tab on the same SSH connection, showing ls -la of the project folder while the upload keeps running below."
        lazy
      />
    </section>
  );
}
