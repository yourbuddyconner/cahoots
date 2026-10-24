// Types the sketch's functions take as parameters. They live here because the Arduino build inserts
// function prototypes above the sketch body, before any type the sketch itself declares.
#pragma once
#include <stdint.h>

struct Btn { uint8_t pin; const char* name; bool down; uint32_t t; };
enum Screen { SCREEN_WORDS, SCREEN_QR6, SCREEN_QR5, SCREEN_WAITING };
