package dev.nori.music.data

/**
 * A command a screen has drawn ahead of its answer: [asked] is what it drew, [ticket] the command's number
 * ([dev.nori.music.playback.PlayerConnection.ticket]). A state that has answered the command, whatever
 * came of it (done, refused, overtaken), carries an answered count at or past the ticket; the screen holds
 * [asked] until then, not for a guessed time.
 */
class Pending<out T>(val asked: T, val ticket: Long) {
    fun answeredBy(answered: Long): Boolean = answered >= ticket
}
