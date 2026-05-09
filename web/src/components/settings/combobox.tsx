"use client";

import { useState, type ReactNode } from "react";
import { useCombobox } from "downshift";

interface ComboboxItem {
  value: string;
  label: string;
}

interface ComboboxInputProps {
  label: ReactNode;
  value: string;
  items: ComboboxItem[];
  onChange: (value: string) => void;
  onBlur?: () => void;
  placeholder?: string;
  allowFreeText?: boolean;
  disabled?: boolean;
}

export function ComboboxInput({
  label,
  value,
  items,
  onChange,
  placeholder,
  allowFreeText = true,
  onBlur,
  disabled = false,
}: ComboboxInputProps) {
  const [filteredItems, setFilteredItems] = useState(items);
  const [prevItemsLen, setPrevItemsLen] = useState(items.length);

  if (items.length !== prevItemsLen) {
    setPrevItemsLen(items.length);
    setFilteredItems(items);
  }

  const {
    isOpen,
    getToggleButtonProps,
    getLabelProps,
    getMenuProps,
    getInputProps,
    getItemProps,
    highlightedIndex,
  } = useCombobox({
    items: filteredItems,
    inputValue: value,
    itemToString: (item) => item?.value ?? "",
    onInputValueChange: ({ inputValue, type }) => {
      if (type === useCombobox.stateChangeTypes.InputChange) {
        const query = (inputValue ?? "").toLowerCase();
        setFilteredItems(
          query
            ? items.filter(
                (item) =>
                  item.value === value ||
                  item.label.toLowerCase().includes(query) ||
                  item.value.toLowerCase().includes(query)
              )
            : items
        );
        if (allowFreeText) {
          onChange(inputValue ?? "");
        }
      }
    },
    onSelectedItemChange: ({ selectedItem }) => {
      if (selectedItem) {
        onChange(selectedItem.value);
        setFilteredItems(items);
      }
    },
    onIsOpenChange: ({ isOpen: nowOpen }) => {
      if (nowOpen) {
        setFilteredItems(items);
      }
    },
    stateReducer: (_state, actionAndChanges) => {
      const { changes, type } = actionAndChanges;
      if (
        type === useCombobox.stateChangeTypes.InputBlur ||
        type === useCombobox.stateChangeTypes.InputKeyDownEscape
      ) {
        return { ...changes, inputValue: value };
      }
      return changes;
    },
  });

  return (
    <div className="space-y-1">
      <label
        className="flex items-center gap-2 text-sm font-medium text-text-secondary"
        {...getLabelProps()}
      >
        {label}
      </label>
      <div className="relative">
        <div className="flex">
          <input
            {...getInputProps({
              onBlur,
            })}
            placeholder={placeholder}
            disabled={disabled}
            className={`w-full rounded-lg border border-border bg-surface px-3 py-2 pr-8 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
          />
          <button
            type="button"
            {...getToggleButtonProps()}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-text-tertiary hover:text-text-primary"
            aria-label="toggle menu"
          >
            <svg
              className={`h-4 w-4 transition-transform ${isOpen ? "rotate-180" : ""}`}
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M19 9l-7 7-7-7"
              />
            </svg>
          </button>
        </div>
        <ul
          {...getMenuProps()}
          className={`absolute z-10 mt-1 w-full max-h-60 overflow-y-auto rounded-lg border border-border bg-surface shadow-lg ${
            !(isOpen && filteredItems.length > 0) ? "hidden" : ""
          }`}
        >
          {isOpen &&
            filteredItems.map((item, index) => (
              <li
                key={item.value}
                {...getItemProps({ item, index })}
                className={`px-3 py-2 text-sm cursor-pointer ${
                  highlightedIndex === index
                    ? "bg-surface-tertiary text-text-primary"
                    : "text-text-primary"
                }`}
              >
                {item.label}
              </li>
            ))}
        </ul>
      </div>
    </div>
  );
}
