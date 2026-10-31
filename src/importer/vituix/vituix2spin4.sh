#!/bin/sh

cp *hor\ 0.txt "../$SPEAKER _H 0.txt"
cp *ver\ 0.txt "../$SPEAKER _V 0.txt"
for i in *hor\ \-*.txt; do cp "$i" "../$SPEAKER _H -${i#Aalto 7 passive hor -}"; done
for i in *ver\ \-*.txt; do cp "$i" "../$SPEAKER _V -${i#Aalto 7 passive ver -}"; done
for i in *hor\ *.txt; do cp "$i" "../$SPEAKER _H ${i#Aalto 7 passive hor }"; done
for i in *ver\ *.txt; do cp "$i" "../$SPEAKER _V ${i#Aalto 7 passive ver }"; done
