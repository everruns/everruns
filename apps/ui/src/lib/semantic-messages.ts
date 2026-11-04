// Message IDs retain grammatical context when an English word is also a command or permission label.
import type { MessageKey } from "./i18n";
import type { SupportedLocale } from "./locale-registry";

export const semanticMessages: Partial<
  Record<SupportedLocale, Partial<Record<MessageKey, string>>>
> = {
  fr: {
    read_label_one: "lecture",
    read_label_many: "lectures",
    search_label_one: "recherche",
    search_label_many: "recherches",
    write_label_one: "écriture",
    write_label_many: "écritures",
    shell_label_one: "commande",
    shell_label_many: "commandes",
    tool_label_one: "outil",
    tool_label_many: "outils",
  },
  de: {
    read_label_one: "Lesevorgang",
    read_label_many: "Lesevorgänge",
    search_label_one: "Suche",
    search_label_many: "Suchen",
    write_label_one: "Schreibvorgang",
    write_label_many: "Schreibvorgänge",
    shell_label_one: "Befehl",
    shell_label_many: "Befehle",
    tool_label_one: "Werkzeug",
    tool_label_many: "Werkzeuge",
  },
  es: {
    read_label_one: "lectura",
    read_label_many: "lecturas",
    search_label_one: "búsqueda",
    search_label_many: "búsquedas",
    write_label_one: "escritura",
    write_label_many: "escrituras",
    shell_label_one: "comando",
    shell_label_many: "comandos",
    tool_label_one: "herramienta",
    tool_label_many: "herramientas",
  },
  "zh-CN": {
    read_label_one: "次读取",
    read_label_many: "次读取",
    search_label_one: "次搜索",
    search_label_many: "次搜索",
    write_label_one: "次写入",
    write_label_many: "次写入",
    shell_label_one: "次命令执行",
    shell_label_many: "次命令执行",
    tool_label_one: "个工具",
    tool_label_many: "个工具",
  },
};
