import Connection from "./sections/Connection";
import Faq from "./sections/Faq";
import Fast from "./sections/Fast";
import Filezilla from "./sections/Filezilla";
import Footer from "./sections/Footer";
import Hero from "./sections/Hero";
import Install from "./sections/Install";
import Nav from "./sections/Nav";
import OpenSource from "./sections/OpenSource";

export default function App() {
  return (
    <>
      <Nav />
      <main>
        <Hero />
        <Fast />
        <Connection />
        <Filezilla />
        <OpenSource />
        <Install />
        <Faq />
      </main>
      <Footer />
    </>
  );
}
