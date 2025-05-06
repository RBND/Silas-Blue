from colorama import Fore, Style, Back

# Available Colorama constants if you want to modify your theme.
# Fore: BLACK, RED, GREEN, YELLOW, BLUE, MAGENTA, CYAN, WHITE, RESET.
# Back: BLACK, RED, GREEN, YELLOW, BLUE, MAGENTA, CYAN, WHITE, RESET.
# Style: DIM, NORMAL, BRIGHT, RESET_ALL

# Define retrowave theme colors
class RetroColors:
    # Main colors
    BLUE = Fore.BLUE + Style.BRIGHT
    DEEP_BLUE = Fore.BLUE
    MAGENTA = Fore.MAGENTA + Style.BRIGHT
    PURPLE = Fore.MAGENTA
    CYAN = Fore.CYAN + Style.BRIGHT

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
