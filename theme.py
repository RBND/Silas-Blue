from colorama import Fore, Style, Back


class RetroColors:
    # Main colors
    BLUE = Fore.BLUE + Style.BRIGHT
    DEEP_BLUE = Fore.BLUE
    MAGENTA = Fore.MAGENTA + Style.BRIGHT
    PURPLE = Fore.MAGENTA
    CYAN = Fore.CYAN + Style.BRIGHT

    # Backgrounds
    BG_BLUE = Back.BLUE
    BG_MAGENTA = Back.MAGENTA

    # Text styles
    BOLD = Style.BRIGHT
    NORMAL = Style.NORMAL
    DIM = Style.DIM

    # Combinations
    HEADER = MAGENTA + BOLD
    TITLE = CYAN + BOLD
    PROMPT = BLUE + BOLD
    SUCCESS = CYAN + BOLD
    ERROR = MAGENTA + BOLD
    WARNING = PURPLE + BOLD
    INFO = DEEP_BLUE + BOLD
    COMMAND = BLUE
    RESPONSE = PURPLE

    # Reset
    RESET = Style.RESET_ALL
