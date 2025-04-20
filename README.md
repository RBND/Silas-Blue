# Silas Blue 
A discord bot that will allow users to send prompts to a locally run LLM via ollama through Discord messages.

## Requirements:
- Python3 (https://www.python.org/downloads)
- Ollama (https://ollama.com/download)

## Getting Started:
 - **Start Ollama**

  Verify that Ollama is running by opening a terminal and typing `ollama --version`.  
  Ollama will return "ollama version is x.x.x". If it is not running you can start it with the command `ollama serve`.

 - **Verify required Python3 libraries are installed.** </ins> (Discord, aiohttp, asyncio)
  
    Open a terminal and type:

  Windows (Open as Admin)
  > py -3 -m pip install -U discord.py aiohttp asyncio

  Linux  
  > sudo python3 -m pip install -U discord.py aiohttp asyncio

 - Insert your Discord private bot token into bot.py
  Open bot.py in your IDE or text editor and on the very last line of code look for
  >\# Replace 'YOUR_DISCORD_BOT_TOKEN' with your actual bot token  
  >bot.run(os.environ.get('DISCORD_TOKEN', 'INSERT_YOUR_PRIVATE_DISCORD_TOKEN_HERE'))

  Replace `'INSERT_YOUR_PRIVATE_DISCORD_TOKEN_HERE'` with your private key. Make sure you keep the single `'` before and after the key.

## Adding Models
  In your terminal use the command `ollama pull` followed by the model name. For example to download Gemma3:1b you would type `ollama pull gemma3:1b`. You can find more models at https://ollama.com/search

## Running the bot  
  To launch the bot open a terminal and `cd` to the folder you downloaded bot.py into. If you're on windows and have it saved to your desktop this would look like `cd "C:\Users\YOUR_USERNAME\Desktop"`

  Once the bot is running you will a few lines in your terminal about the bot connecting.
  >INFO     discord.client logging in using static token  
  >INFO     discord.gateway Shard ID None has connected to Gateway (Session ID: xxxxxxxxxxxxxxxxx).  
  >Logged in as YOUR_BOT_USERNAME (xxxxxxxxxx)

  After you have your bot join a server it will also display information about the settings saved in the config file for any servers its connected to.
  >Server: YOUR_SERVER_NAME (ID: xxxxxxxxxxxxxxxx)  
  >\- Allowed Models: All  
  >\- Set Model Permission: 2 roles  
  >\- Manage Config Permission: 1 roles  
  >\- Reply To Permission: Everyone  
  >\- Bot Nickname: Default  
  >\- Random Replies: Disabled  
  >\- System Instructions: None  
  >\- Paginated Responses: Enabled  
  >\- Page Size: 750 characters  
  >  
  >\------

## Controlling the bot  
As of V1.1.0 you can issue commands in the terminal to configure settings on per server basis. Type `Help` in the terminal for a list of commands and how to use them.

In Discord you can called the bot with either `!` or tagging it's username. For example both of these will pull up the help info to learn the bots discord commands: `!help` or `@BotName help`


## Contact
You can contact RobotsNeverDie via the Mobile Repair Discord. https://discord.gg/HvM9thmd
