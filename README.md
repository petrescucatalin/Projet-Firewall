[![Review Assignment Due Date](https://classroom.github.com/assets/deadline-readme-button-24ddc0f5d75046c5622901739e7c5dd533143b0c8e959d652212380cedb1ea36.svg)](https://classroom.github.com/a/8StzC9tj)
Proiectul reprezinta un server asincron care manageuieste request-urile GET respectiv POST. Pentru orice alta metoda returneaza un cod 405 (Method not allowed). 
Pentru GET, atunci cand nu se ruleaza un script (.sh), listeaza fisierele directorului sau fisierul in sine (static). Pentru GET, atunci cand se ruleaza un script, serverul executa script-ul si da ca raspuns output-ul fisierului. 
Pentru POST, serverul ruleaza un script din folderul "/scripts" iar body-ul post-ului se da script-ului cu functia stdin. 

Serverul trimite un raspuns HTTP cu status line, headers si body. Are header-uri "Content-Type" respectiv "Content-length". Pentru erori, afiseaza un body html in care include starea erorii si definita ei. 
De notat aici este faptul ca pentru eroarea "500 (Internal Server Error)" serverul nu mai returnează folosind headere "custom" ("if status_code != 500") intrucat nu este compatibil cu formatul testului (fail.sh)

Proiectul foloseste craturile std, respectiv tokio pentru a face un server asincron. Mai specific tokio pentru probleme asincrone si std pentru functionalitati.
In prima functie, in main, preia de la tastatura argumentele si pt fiecare TcpListener creeaza o intrare (server asincron).

A doua functie (handle) este functia care se ocupa practic de tot programul si gestioneaza fiecare request. Preia socket-ul si ip-ul clientului si imparte date intre sender si reciever si trimite un raspuns conform task-ului, o jumatate sa citeasca request-ul si headerele, iar a doua sa trimita raspunsul la client.

Am creat pentru a "usura" munca si a nu aglomera prea tare functia handle mai multe functii ajutatoare, precum:

is_readable = verifica daca serverul are permisiuni pentru a citi un fisier (pentru testul secret)

split_query_String = teoretic imparte intr-un path, path-ul fisierului si parametrii query

get_content_type = intrucat pentru acest task nu am voie sa folosesc alte crate-uri externe (mime_guess), am creat o functie care afla tipul fisierului.

get_status = unde am notat pentru fiecare cod de eroare ce reprezinta ca in cerintele task-ului, ca sa nu scriu la fiecare in parte, sa fie mai aerisit codul

write_response = scrie raspunsul HTTP catre partea de client, status line, headere si body

run_script = ruleaza script-ul si preia rezultatul, pentru GET si POST. aici a fost mai complicat putin, dar in concluzie, functia ia detaliile request-ului, verifica script-ul, citeste headerurile si body-ul requestului, executa scriptul si returneaza scriptul cu codul specific. Returneaza fie un tuplu (Vec<u8>) si un u16, fie o eroare.
