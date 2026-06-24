import React, { useEffect, useRef } from "react";

interface TranscriptionDisplayProps {
  japaneseText: string;
  vietnameseText: string;
}

interface TextSectionProps {
  title: string;
  text: string;
  placeholder: string;
  sectionClassName: string;
  textBoxClassName: string;
  isStreaming?: boolean;
}

const TextSection: React.FC<TextSectionProps> = ({
  title,
  text,
  placeholder,
  sectionClassName,
  textBoxClassName,
  isStreaming = false,
}) => {
  const textBoxRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when text updates (streaming mode)
  useEffect(() => {
    if (textBoxRef.current && isStreaming && text) {
      textBoxRef.current.scrollTop = textBoxRef.current.scrollHeight;
    }
  }, [text, isStreaming]);

  return (
    <div className={`text-section ${sectionClassName}`}>
      <h3 className="section-title">
        {title}
        {isStreaming && text && <span className="streaming-indicator">● streaming</span>}
      </h3>
      <div 
        ref={textBoxRef}
        className={`text-box ${textBoxClassName}`}
      >
        {text || <span className="placeholder">{placeholder}</span>}
      </div>
    </div>
  );
};

const TranscriptionDisplay: React.FC<TranscriptionDisplayProps> = ({
  japaneseText,
  vietnameseText,
}) => {
  const isStreaming = japaneseText.length > 0;

  return (
    <div className="transcription-display">
      <TextSection
        title="Japanese"
        text={japaneseText}
        placeholder="Japanese text will appear here..."
        sectionClassName="japanese-section"
        textBoxClassName="japanese-text"
        isStreaming={isStreaming}
      />

      <div className="divider"></div>

      <TextSection
        title="Vietnamese"
        text={vietnameseText}
        placeholder="Vietnamese translation will appear here..."
        sectionClassName="vietnamese-section"
        textBoxClassName="vietnamese-text"
        isStreaming={isStreaming}
      />
    </div>
  );
};

export default TranscriptionDisplay;
