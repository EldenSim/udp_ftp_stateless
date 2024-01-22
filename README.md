# UDP File Transfer Protocol (UDP FTP)

The UDP FTP was developed for the purpose of transferring files from one machine to another through a one-way data diode. The following documents how to set up the programme, some of its limitation and areas of improvement. As a novice in Rust, there may be some mistakes in optimisation in the code.

NOTE: UDP FTP does not support nested folders to be sent, so for nested folders please tar or compress the folder before sending over. (Possible feature to be added in the future)

# Error Correction

Error Correction is used as packets may be lost during data tranmission through UDP. Thus, Raptor Codes are used as FEC.
Raptor Codes was chosen as it is a fountain code, in which x amount of packets need to be received for the decoder to reconstruct the original data. This means the receiver does not need to receive packets in order (i.e Packet 1 then Packet 2).

For improvements:

-   Use RaptorQ error correction code (Faster encoding and decoding times)

# Environment Variables

```
FTP_MODE=recv
LOCAL_ADDRESS=127.0.0.1
FOREIGN_ADDRESS=127.0.0.1
SENDING_DIRECTORY=./sending_dir
RECEIVING_DIRECTORY=./receiving_dir
```

-   `FTP_MODE`: send / recv (sets programme into respective mode)
-   `LOCAL_ADDRESS`: Valid IPv4 address (local machine's IPv4 address)
-   `FOREIGN_ADDRESS`: Valid IPv4 address (receiving machine's IPv4 address)
-   `SENDING_DIR`: Directory for files to be sent
-   `RECEIVING_DIRECTORY`: Directory for final files to be written after processing

```
MTU=9000
MAX_CHUNKSIZE=500000
MAX_SOURCE_SYMBOL_SIZE=64
NUMBER_OF_REPAIR_SYMBOLS=128
DELAY_PER_CHUNK=40
DELAY_PER_FILE=0
PROCESSING_STORAGE=256
```

-   `MTU`: Maximum Transmission Limit (Limits packet size)
    -   Maximum packet size that can be trasmitted through the network.
-   `MAX_CHUNKSIZE`: Max chunksize for a large file to be split into to fit `MTU`

    -   After encoding, encoded symbols will be larger than original symbols, thus we will limit the size of the chunk before encoding to ensure that the encoded symbol will fit the `MTU` limit
    -   E.g. For a `MTU` = 9000, the maximum chunksize is `MTU * MAX_SOURCE_SYMBOL_SIZE = 9000 * 64 = 576_000`. But packet preamble takes some space so we can limit chunksize to 500_000

-   `MAX_SOURCE_SYMBOL_SIZE`: Raptor Code variable
    -   Larger the symbol size, the smaller the encoded symbols size
-   `NUMBER_OF_REPAIR_SYMBOLS`: Raptor Code variable

    -   Increases the number of symbols sent, so even if more packets are lost, receiver is still able to reconstruct data
    -   E.g. Total symbols sent = 256 packets, I only need to receive 128 packets to reconstruct the data

-   `DELAY_PER_CHUNK`: Delay sender from sending after every Chunk (Default: 40 for reliable file transfer)

-   `DELAY_PER_FILE`: Delay sender from sending after every File

-   `PROCESSING_STORAGE`: Receiver will wait for this amount of packets before processing (Default: 256)

# Sender programme logic

1. Bind UDPSocket to IP address
2. Connect Recv IP address to UDPSocket
3. Calculate max chunksize to split file into
4. Loop through each file in `SENDING_DIRECTORY`
    1. Break files into chunks
    2. Initialise Raptor Encoder
    3. Loop through total number of symbols (num of source symbol + num of repair symbol)
        1. Obtain the encoding_symbols (Vec<u8>)
        2. Initialise Preamble and Packet
        3. Serialise Packet into bytes
        4. Send Packet
    4. Sleep for `DELAY_PER_CHUNK` time
    5. Calculate number of empty Packets to send
    6. Initialise empty Packets
    7. Serialise empty Packet
    8. Loop for number of empty Packets to send
        1. Send empty Packets
5. Programme end

# Receiver programme logic

This is a rough breakdown of

1. Bind UDPSocket to IP address
2. Within loop
    1. Initialise storage vectors
        - `file_chunk_segment`: Keep track of number of segments received
        - `file_data`: Keeps segment data only
        - `decoding_status`: Keeps track of decoding status of each chunk
        - `merging_status`: Keeps track of merging status of chunks
        - `files_to_be_merged`: Keeps track of files that have not been merged
    1. Listen for incomming Packets
    1. Store received Packets into `storage`
    1. Check if storage contains enough Packets (`PROCESSING_STORAGE`), if not enough skip this iteration (go to step 2.1)
    1. Drain Packets in `storage` into `processing_storage`
    1. Clone the storage vectors
    1. Spawn a task to parse the Packets
        1. Loop through the packets in `processing_storage`
            1. Check if Packet is an empty Packet if so skip this Packet
            1. Check if file already exist in `receiving_dir` and filesize matches, if so skip this Packet
            1. Check if file is in `files_to_be_merged`, add if not
            1. Add `FileData` struct with filename and empty `chunk_segment_data` vec
            1. Add `ChunkSegmentData` struct with `chunk_segment_name` and packet `data`.
            1. Add `FileChunkSegment` into `file_chunk_segment` and increment the counter for the chunk segment by 1
            1. Obtain number of segment received
            1. Add `DecodingStatus` into `decoding_status` with "Undecoded" if new chunk
            1. Check if received segments is enough for the decoder.
            1. Collect the segments into another vec
            1. Change chunk status in `decoding_status` to "Decoding"
            1. Spawn a new task to handle the decoding of segments
                1. Try to decode chunk
                1. If successful, write chunk into file and store in `./temp/`, change `chunk_status` to "Decoded" and add `MergingStatus` struct to `merging_status`
                1. If not successful, change chunk_status to "Undecoded"
    1. Clone storage vector again
    1. Spawn a task to handle merging of files
        1. Try to merge file using `merge_temp_files`
        1. If successful, remove finished file data from storage vectors
        1. If not successful, return from task and continue loop

# Possible Areas of Improvement

-   Handling of nested folder structure (i.e Able to retain folder structure during transmission)
-   Use of different FEC to speed up encoding and decoding times
    -   RaptorQ
-   Optimising Receiver logic
    -   Compact the `storages` vec into one vec with a struct that contains all information
-   More efficient handling of shared memory on heap, instead of `Arc<Mutex<T>>` types
