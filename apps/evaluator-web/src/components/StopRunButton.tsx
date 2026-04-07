interface StopRunButtonProps {
  disabled: boolean;
  isStopping: boolean;
  onStop: () => void;
}

export function StopRunButton({
  disabled,
  isStopping,
  onStop
}: StopRunButtonProps) {
  return (
    <button
      className="stop-button"
      disabled={disabled || isStopping}
      onClick={onStop}
      type="button"
    >
      {isStopping ? "Stopping run..." : "Stop Run"}
    </button>
  );
}
