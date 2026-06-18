import { useState } from "react";

type Props = {
  placeholder: string;
  onAttach: () => void;
  onSend: (value: string) => void;
};

export function Composer(props: Props) {
  const [value, setValue] = useState("");

  function submit() {
    const trimmed = value.trim();
    if (!trimmed) return;
    props.onSend(trimmed);
    setValue("");
  }

  return (
    <footer className="composer">
      <button className="icon-button" onClick={props.onAttach}>＋</button>
      <input
        value={value}
        placeholder={props.placeholder}
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") submit();
        }}
      />
      <button className="send-button" onClick={submit}>发送</button>
    </footer>
  );
}
