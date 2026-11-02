import mark from "../../../../assets/brand/honk-mark.svg";
import darkMark from "../../../../assets/brand/honk-mark-dark.svg";
import "./BrandLogo.css";

export default function BrandLogo() {
  return (
    <span
      className="brand-logo inline-flex size-9 shrink-0 items-center justify-center"
      aria-hidden="true"
    >
      <img
        src={mark}
        alt=""
        width={36}
        height={36}
        className="size-9 dark:hidden"
        data-brand="light"
      />
      <img
        src={darkMark}
        alt=""
        width={36}
        height={36}
        className="hidden size-9 dark:block"
        data-brand="dark"
      />
    </span>
  );
}
