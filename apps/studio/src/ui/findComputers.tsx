import { useState } from "preact/hooks";
import { pairWithCode, type Pairing } from "../core/pairing";
import { remoteFetcher } from "../shell/remoteFetch";
import type { Shell } from "../shell/shell";
import { describe } from "../sources/transport";

interface Found {
  name: string;
  url: string;
}

interface Props {
  shell: Shell;
  disabled: boolean;
  onPaired(pairing: Pairing): void;
}

/**
 * In the apps: computers on this network that let other devices use their engine (they announce themselves while
 * that is on). Choosing one asks for the six-digit code the computer shows, which gives the token.
 */
export function FindComputers({ shell, disabled, onPaired }: Props) {
  const [found, setFound] = useState<Found[] | null>(null);
  const [looking, setLooking] = useState(false);
  const [chosen, setChosen] = useState<Found | null>(null);
  const [code, setCode] = useState("");
  const [problem, setProblem] = useState("");
  const [pairing, setPairing] = useState(false);

  const look = () => {
    setLooking(true);
    setProblem("");
    setChosen(null);
    shell
      .bridge<Found[]>("find_engines")
      .then(setFound, (error) => setProblem(describe(error)))
      .finally(() => setLooking(false));
  };
  const pair = (event: Event) => {
    event.preventDefault();
    if (chosen === null) return;
    setPairing(true);
    setProblem("");
    pairWithCode(remoteFetcher(shell), chosen.url, code)
      .then(onPaired, (error) => setProblem(describe(error)))
      .finally(() => setPairing(false));
  };

  return (
    <div class="find-computers">
      <button type="button" class="button" disabled={disabled || looking} onClick={look}>
        {looking ? "Looking..." : "Find computers on this network"}
      </button>
      {found !== null && found.length === 0 && !looking && (
        <p class="help">
          None found. On the computer: Connection, This computer, "Let other devices use this computer"; both on the same
          network. Or scan its code.
        </p>
      )}
      {found !== null && found.length > 0 && (
        <ul class="found-computers">
          {found.map((computer) => (
            <li key={computer.url}>
              <button
                type="button"
                class={chosen?.url === computer.url ? "button primary" : "button"}
                onClick={() => {
                  setChosen(computer);
                  setCode("");
                  setProblem("");
                }}
              >
                {computer.name}
              </button>{" "}
              <span class="mono help">{computer.url}</span>
            </li>
          ))}
        </ul>
      )}
      {chosen !== null && (
        <form class="pair-code" onSubmit={pair} novalidate>
          <label class="field">
            <span class="field-label">Pairing code of {chosen.name}</span>
            <input
              type="text"
              inputMode="numeric"
              autocomplete="one-time-code"
              maxLength={7}
              placeholder="123456"
              value={code}
              onInput={(event) => setCode(event.currentTarget.value)}
            />
            <span class="help">Six digits, shown on the computer under "Let other devices use this computer".</span>
          </label>
          <button type="submit" class="button primary" disabled={pairing}>
            {pairing ? "Pairing..." : "Pair"}
          </button>
        </form>
      )}
      {problem !== "" && (
        <p class="field-error" role="alert">
          {problem}
        </p>
      )}
    </div>
  );
}
